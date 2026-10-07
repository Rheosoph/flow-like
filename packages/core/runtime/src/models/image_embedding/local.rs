#![cfg(feature = "local-ml")]
use crate::{
    bit::{Bit, BitTypes},
    models::{
        embedding::{PREPARATION_SLOTS, run_embedding_session},
        embedding_factory::EmbeddingFactory,
        local_utils::ensure_local_weights,
    },
    state::FlowLikeState,
};
use flow_like_model_provider::embedding::native::{
    ImagePreprocessor, NativeImageEmbedding, SessionOptions,
};
use flow_like_model_provider::{
    embedding::{EmbeddingModelLogic, GeneralTextSplitter},
    image_embedding::ImageEmbeddingModelLogic,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::tokio::sync::Semaphore;
use flow_like_types::{Cacheable, Result, async_trait, sync::Mutex};
use image::DynamicImage;
use std::{any::Any, sync::Arc};

#[derive(Clone)]
pub struct LocalImageEmbeddingModel {
    pub bit: Arc<Bit>,
    image_embedding_model: Arc<Mutex<NativeImageEmbedding>>,
    preprocessor: ImagePreprocessor,
    preparation_slots: Arc<Semaphore>,
    output_dimensions: Option<usize>,
    text_model: Arc<dyn EmbeddingModelLogic>,
}

impl Cacheable for LocalImageEmbeddingModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl LocalImageEmbeddingModel {
    pub async fn new(
        bit: &Bit,
        app_state: Arc<FlowLikeState>,
        factory: &EmbeddingFactory,
    ) -> flow_like_types::Result<Arc<Self>> {
        let bit = Arc::new(bit.clone());
        let bit_store = FlowLikeState::bit_store(&app_state).await?;

        let bit_store = match bit_store {
            FlowLikeStore::Local(store) => store,
            _ => return Err(flow_like_types::anyhow!("Only local store supported")),
        };

        let pack = bit.pack(app_state.clone()).await?;
        ensure_local_weights(&pack, &app_state, bit.id.as_str(), "image embedding model").await?;
        let embedding_model = pack
            .bits
            .iter()
            .find(|b| b.bit_type == BitTypes::Embedding)
            .ok_or(flow_like_types::anyhow!("Embedding model not found."))?;
        let preprocessor_bit = pack
            .bits
            .iter()
            .find(|b| b.bit_type == BitTypes::PreprocessorConfig)
            .ok_or(flow_like_types::anyhow!("Preprocessor bit not found."))?;
        let text_model = factory
            .build_text(embedding_model, app_state.clone())
            .await?;

        let model_path = bit
            .to_path(&bit_store)
            .ok_or(flow_like_types::anyhow!("No model path"))?;

        let preprocessor_path = preprocessor_bit
            .to_path(&bit_store)
            .ok_or(flow_like_types::anyhow!("No model path"))?;
        flow_like_types::tokio::task::spawn_blocking(move || {
            let loaded_preprocessor = std::fs::read(preprocessor_path)?;
            Self::from_installed_assets(bit, &model_path, &loaded_preprocessor, text_model)
        })
        .await
        .map_err(|error| {
            flow_like_types::anyhow!("Image embedding model loader task failed: {error}")
        })?
    }

    fn from_installed_assets(
        bit: Arc<Bit>,
        model_path: &std::path::Path,
        preprocessor: &[u8],
        text_model: Arc<dyn EmbeddingModelLogic>,
    ) -> Result<Arc<Self>> {
        let loaded_model = NativeImageEmbedding::new_from_file(
            model_path,
            preprocessor,
            SessionOptions::default(),
        )?;

        let default_return_model = LocalImageEmbeddingModel {
            bit,
            output_dimensions: loaded_model.output_dimensions(),
            preprocessor: loaded_model.preprocessor().clone(),
            preparation_slots: Arc::new(Semaphore::new(PREPARATION_SLOTS)),
            image_embedding_model: Arc::new(Mutex::new(loaded_model)),
            text_model,
        };

        Ok(Arc::new(default_return_model))
    }
}

#[async_trait]
impl ImageEmbeddingModelLogic for LocalImageEmbeddingModel {
    fn output_dimensions(&self) -> Option<usize> {
        self.output_dimensions
    }
    fn context_tokens(&self) -> Option<usize> {
        self.text_model.context_tokens()
    }
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> flow_like_types::Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        return self.text_model.get_splitter(capacity, overlap).await;
    }

    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        return self.text_model.text_embed_query(texts).await;
    }

    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        return self.text_model.text_embed_document(texts).await;
    }

    async fn image_embed(&self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.image_embedding_model.clone();
        let preprocessor = self.preprocessor.clone();
        let permit = self.preparation_slots.clone().acquire_owned().await?;
        let embeddings = flow_like_types::tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let started = std::time::Instant::now();
            let pixels = preprocessor.preprocess_batch(images)?;
            tracing::debug!(
                preprocessing_ms = started.elapsed().as_secs_f64() * 1000.0,
                "image embedding preprocessing"
            );
            run_embedding_session(&model, |model| model.embed_pixels(&pixels))
        })
        .await
        .map_err(|e| flow_like_types::anyhow!("Blocking task failed: {}", e))?
        .map_err(|e| flow_like_types::anyhow!("Error embedding image: {}", e))?;

        Ok(embeddings)
    }

    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::embedding::local::LocalEmbeddingModel;
    use flow_like_model_provider::embedding::{
        adapters::LegacyEmbeddingAdapter,
        interface::{
            EmbeddingDescriptor, EmbeddingInput, EmbeddingLimits, EmbeddingMetric,
            EmbeddingModality, EmbeddingModel, EmbeddingOptions, EmbeddingPurpose,
            EmbeddingRequest, EmbeddingSpace,
        },
        native::TokenizerFiles,
    };
    use flow_like_types::tokio;
    use std::path::PathBuf;

    #[tokio::test]
    #[ignore = "Requires all published model assets in FLOW_LIKE_EMBEDDING_MODELS"]
    async fn runtime_and_modern_image_apis_match_published_golden_vectors() -> Result<()> {
        let assets = PathBuf::from(std::env::var_os("FLOW_LIKE_EMBEDDING_MODELS").ok_or_else(
            || flow_like_types::anyhow!("Set FLOW_LIKE_EMBEDDING_MODELS to model assets"),
        )?);
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../model-provider/tests/fixtures/embedding_parity");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixtures.join("manifest.json"))?)?;
        let entries = manifest["models"].as_array().unwrap();
        let inputs: Vec<String> = serde_json::from_value(manifest["inputs"].clone())?;
        let image_specs: Vec<(u32, u32, u32)> =
            serde_json::from_value(manifest["image_fixtures"].clone())?;
        let images: Vec<DynamicImage> = image_specs
            .iter()
            .map(|&(width, height, seed)| {
                DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| {
                    image::Rgb([
                        ((x * 3 + y + seed * 41) % 256) as u8,
                        ((x + y * 5 + seed * 23) % 256) as u8,
                        ((x * 7 + y * 11 + seed * 13) % 256) as u8,
                    ])
                }))
            })
            .collect();
        let mut models = 0;
        let mut comparisons = 0;
        let mut maximum: f64 = 0.0;
        for entry in entries
            .iter()
            .filter(|entry| entry["bit"]["type"] == "ImageEmbedding")
        {
            let bit = Arc::new(serde_json::from_value::<Bit>(entry["bit"].clone())?);
            let text_entry = entries
                .iter()
                .find(|candidate| {
                    candidate["bit"]["type"] == "Embedding"
                        && bit.dependencies.iter().any(|dependency| {
                            dependency.rsplit(':').next() == candidate["bit"]["id"].as_str()
                        })
                })
                .ok_or_else(|| flow_like_types::anyhow!("Missing paired text Bit"))?;
            let text_bit = Arc::new(serde_json::from_value::<Bit>(text_entry["bit"].clone())?);
            let text_dir = assets.join(&text_bit.id);
            let read = |name| std::fs::read(text_dir.join(name));
            let text = LocalEmbeddingModel::from_installed_assets(
                text_bit,
                &text_dir.join("model.onnx"),
                TokenizerFiles {
                    tokenizer_file: read("tokenizer.json")?,
                    config_file: read("config.json")?,
                    special_tokens_map_file: read("special_tokens_map.json")?,
                    tokenizer_config_file: read("tokenizer_config.json")?,
                },
                None,
            )?;
            let directory = assets.join(&bit.id);
            let model = LocalImageEmbeddingModel::from_installed_assets(
                bit.clone(),
                &directory.join("model.onnx"),
                &std::fs::read(directory.join("preprocessor_config.json"))?,
                text,
            )?;
            let cases = entry["cases"].as_array().unwrap();
            let dimensions = cases[0]["dimensions"].as_u64().unwrap() as usize;
            assert_eq!(model.output_dimensions(), Some(dimensions));
            let modern = LegacyEmbeddingAdapter::image(
                model.clone(),
                EmbeddingDescriptor {
                    model_id: bit.id.clone(),
                    adapter: "legacy_image".into(),
                    modalities: vec![EmbeddingModality::Image, EmbeddingModality::Text],
                    joint_combinations: Vec::new(),
                    purposes: vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document],
                    space: EmbeddingSpace {
                        id: bit.id.clone(),
                        dimensions,
                        normalized: true,
                        metric: EmbeddingMetric::Cosine,
                    },
                    supported_dimensions: vec![dimensions],
                    limits: EmbeddingLimits::default(),
                    pipeline_fingerprint: bit.hash.clone(),
                },
            );
            let golden = fixture_vectors(&fixtures, entry)?;
            for case in cases {
                let vectors = model.image_embed(images.clone()).await?;
                maximum = maximum.max(compare_vectors(case, &vectors, &golden));
                comparisons += vectors.len();
                let output = modern
                    .embed(EmbeddingRequest {
                        items: images.iter().cloned().map(EmbeddingInput::image).collect(),
                        purpose: EmbeddingPurpose::Document,
                        options: EmbeddingOptions::default(),
                    })
                    .await?;
                maximum = maximum.max(compare_vectors(case, &output.embeddings, &golden));
                comparisons += output.embeddings.len();
            }
            let text_golden = fixture_vectors(&fixtures, text_entry)?;
            for (name, purpose) in [
                ("query", EmbeddingPurpose::Query),
                ("document", EmbeddingPurpose::Document),
            ] {
                let case = text_entry["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|case| case["name"] == name)
                    .unwrap();
                let vectors = if purpose == EmbeddingPurpose::Query {
                    model.text_embed_query(&inputs).await?
                } else {
                    model.text_embed_document(&inputs).await?
                };
                maximum = maximum.max(compare_vectors(case, &vectors, &text_golden));
                comparisons += vectors.len();
                let output = modern
                    .embed(EmbeddingRequest::texts(inputs.clone(), purpose))
                    .await?;
                maximum = maximum.max(compare_vectors(case, &output.embeddings, &text_golden));
                comparisons += output.embeddings.len();
            }
            eprintln!("{}: runtime and modern image/text APIs matched", bit.id);
            models += 1;
        }
        assert!(models > 0, "No published image Bits checked");
        eprintln!(
            "{models} image models, {comparisons} vector comparisons, max cosine distance {maximum:.3e}"
        );
        Ok(())
    }

    fn fixture_vectors(fixtures: &std::path::Path, entry: &serde_json::Value) -> Result<Vec<f32>> {
        let bytes = std::fs::read(fixtures.join(entry["vector_file"].as_str().unwrap()))?;
        Ok(bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect())
    }

    fn compare_vectors(case: &serde_json::Value, vectors: &[Vec<f32>], golden: &[f32]) -> f64 {
        let dimensions = case["dimensions"].as_u64().unwrap() as usize;
        let offset = case["offset_floats"].as_u64().unwrap() as usize;
        assert_eq!(vectors.len(), case["rows"].as_u64().unwrap() as usize);
        let mut maximum: f64 = 0.0;
        for (row, vector) in vectors.iter().enumerate() {
            assert_eq!(vector.len(), dimensions);
            let reference = &golden[offset + row * dimensions..offset + (row + 1) * dimensions];
            let mut dot = 0.0;
            let mut left = 0.0;
            let mut right = 0.0;
            for (&value, &expected) in vector.iter().zip(reference) {
                assert!(value.is_finite());
                let value = f64::from(value);
                let expected = f64::from(expected);
                dot += value * expected;
                left += value * value;
                right += expected * expected;
            }
            assert!(left > 0.0 && right > 0.0);
            let distance = (1.0 - dot / (left * right).sqrt()).max(0.0);
            assert!(
                distance <= 1e-5,
                "Cosine distance {distance:.9e} for {} row{row}",
                case["name"]
            );
            maximum = maximum.max(distance);
        }
        maximum
    }
}
