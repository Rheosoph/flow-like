#![cfg(feature = "local-ml")]

use super::{PREPARATION_SLOTS, run_embedding_session};
use std::{any::Any, collections::BTreeMap, path::Path, sync::Arc};

use flow_like_model_provider::{
    embedding::{
        EmbeddingModelLogic, GeneralTextSplitter,
        gemma2::{Gemma2Embedding, Gemma2Options, Gemma2Preprocessor},
        interface::*,
        native::{
            ImagePreprocessor, NativeImageEmbedding, NativeTextEmbedding, NativeTextPreprocessor,
            Pooling, SessionOptions, TokenizedBatch, TokenizerFiles,
        },
    },
    image_embedding::ImageEmbeddingModelLogic,
    text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter},
    tokenizer::TokenizerSizer,
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::tokio::sync::Semaphore;
use flow_like_types::{Cacheable, Result, anyhow, async_trait, sync::Mutex};
use image::DynamicImage;

use crate::{
    bit::{Bit, BitPack},
    models::embedding_factory::versioned_embedding_spec as embedding_spec,
    models::local_utils::ensure_local_weights,
    state::FlowLikeState,
};

#[derive(Clone)]
pub struct LocalMultimodalEmbeddingModel {
    descriptor: EmbeddingDescriptor,
    model: Arc<LocalEmbeddingBackend>,
    preparation_slots: Arc<Semaphore>,
    sizer: TokenizerSizer,
    splitter_capacity: usize,
    // ONNX external weights can remain file-backed for the lifetime of a session.
    _bundle: Arc<tempfile::TempDir>,
}

enum LocalEmbeddingBackend {
    Gemma2 {
        model: Mutex<Gemma2Embedding>,
        preprocessor: Gemma2Preprocessor,
    },
    DualEncoder {
        text: Mutex<NativeTextEmbedding>,
        text_preprocessor: NativeTextPreprocessor,
        image: Option<(Mutex<NativeImageEmbedding>, ImagePreprocessor)>,
        profiles: BTreeMap<EmbeddingPurpose, EmbeddingTaskProfile>,
        max_tokens: usize,
    },
}

impl LocalEmbeddingBackend {
    fn embed(&self, items: &[EmbeddingInput], purpose: EmbeddingPurpose) -> Result<Vec<Vec<f32>>> {
        match self {
            Self::Gemma2 {
                model,
                preprocessor,
            } => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let started = std::time::Instant::now();
                    let prepared = preprocessor.prepare(item, purpose)?;
                    tracing::debug!(
                        index,
                        preprocessing_ms = started.elapsed().as_secs_f64() * 1000.0,
                        "Gemma embedding preprocessing"
                    );
                    run_embedding_session(model, |model| model.embed_prepared(prepared))
                        .map_err(|error| error.context(format!("EmbeddingGemma 2 input {index}")))
                })
                .collect(),
            Self::DualEncoder {
                text,
                text_preprocessor,
                image,
                profiles,
                max_tokens,
            } => {
                let profile = profiles.get(&purpose).cloned().unwrap_or_default();
                let mut texts = Vec::new();
                let mut text_indices = Vec::new();
                let mut images = Vec::new();
                let mut image_indices = Vec::new();
                let started = std::time::Instant::now();
                for (index, item) in items.iter().enumerate() {
                    if item.parts.len() != 1 || item.title.is_some() {
                        return Err(anyhow!(
                            "This dual encoder requires one content part per item without a title"
                        ));
                    }
                    match &item.parts[0] {
                        EmbeddingPart::Text(value) => {
                            let value = format!("{}{}{}", profile.prefix, value, profile.suffix);
                            texts.push(value);
                            text_indices.push(index);
                        }
                        EmbeddingPart::Image(value) if image.is_some() => {
                            images.push(value.as_ref());
                            image_indices.push(index);
                        }
                        part => {
                            return Err(anyhow!(
                                "Dual encoder does not support {:?}",
                                part.modality()
                            ));
                        }
                    }
                }
                let mobile = cfg!(any(
                    target_os = "ios",
                    target_os = "tvos",
                    target_os = "android"
                ));
                let attention_edge = if mobile { 2048usize } else { 4096usize };
                let budget = attention_edge.pow(2);
                let mut cached_encodings = Some(Vec::new());
                let mut cached_tokens = 0;
                let mut token_lengths = Vec::with_capacity(texts.len());
                for value in &texts {
                    let encoding = text_preprocessor.encode_untruncated_one(value)?;
                    let length = encoding.len();
                    if length > *max_tokens {
                        return Err(anyhow!(
                            "Embedding input has {length} tokens, exceeding the {max_tokens}-token context; split the document first"
                        ));
                    }
                    token_lengths.push(length);
                    cached_tokens += length;
                    // A full Encoding also owns token strings and offsets. Large requests
                    // keep the original two-pass path instead of retaining every encoding.
                    if cached_tokens > attention_edge * 16 {
                        cached_encodings = None;
                    }
                    if let Some(encodings) = &mut cached_encodings {
                        encodings.push(encoding);
                    }
                }
                tracing::debug!(
                    preprocessing_ms = started.elapsed().as_secs_f64() * 1000.0,
                    "embedding text tokenization"
                );
                let mut output = vec![Vec::new(); items.len()];
                let mut vectors = Vec::with_capacity(texts.len());
                let mut encodings = cached_encodings.map(Vec::into_iter);
                let mut tokens = TokenizedBatch::default();
                let mut start = 0;
                while start < texts.len() {
                    let mut end = start;
                    let mut longest = 0;
                    while end < texts.len() && end - start < 256 {
                        let candidate = longest.max(token_lengths[end]);
                        if end > start && (end - start + 1) * candidate * candidate > budget {
                            break;
                        }
                        longest = candidate;
                        end += 1;
                    }
                    if let Some(encodings) = &mut encodings {
                        text_preprocessor.pad_encodings_into(
                            encodings.by_ref().take(end - start).collect(),
                            &mut tokens,
                        )?;
                    } else {
                        text_preprocessor.tokenize_into(&texts[start..end], &mut tokens)?;
                    }
                    vectors.extend(run_embedding_session(text, |text| {
                        text.embed_tokens(&tokens)
                    })?);
                    start = end;
                }
                if vectors.len() != text_indices.len() {
                    return Err(anyhow!("Text encoder returned the wrong number of vectors"));
                }
                for (index, vector) in text_indices.into_iter().zip(vectors) {
                    output[index] = vector;
                }
                if let Some((image, preprocessor)) = image {
                    let mut vectors = Vec::with_capacity(image_indices.len());
                    for batch in images.chunks(16) {
                        let started = std::time::Instant::now();
                        let pixels = preprocessor.preprocess_batch_borrowed(batch)?;
                        tracing::debug!(
                            preprocessing_ms = started.elapsed().as_secs_f64() * 1000.0,
                            "image embedding preprocessing"
                        );
                        vectors.extend(run_embedding_session(image, |image| {
                            image.embed_pixels(&pixels)
                        })?);
                    }
                    if vectors.len() != image_indices.len() {
                        return Err(anyhow!(
                            "Image encoder returned the wrong number of vectors"
                        ));
                    }
                    for (index, vector) in image_indices.into_iter().zip(vectors) {
                        output[index] = vector;
                    }
                }
                Ok(output)
            }
        }
    }
}

impl LocalMultimodalEmbeddingModel {
    pub async fn new(bit: &Bit, state: Arc<FlowLikeState>) -> Result<Arc<Self>> {
        let spec =
            embedding_spec(bit)?.ok_or(anyhow!("Bit has no versioned embedding configuration"))?;
        spec.validate()?;
        if !matches!(
            spec.adapter.as_str(),
            "embedding_gemma2" | "sentence_transformer" | "clip"
        ) {
            return Err(anyhow!("Unsupported embedding adapter {}", spec.adapter));
        }
        if spec.adapter == "embedding_gemma2" && spec.dimensions != 768 {
            return Err(anyhow!(
                "EmbeddingGemma 2's native output has 768 dimensions; request reduced dimensions when embedding"
            ));
        }
        if spec.adapter == "embedding_gemma2"
            && (!matches!(spec.pooling, EmbeddingPooling::Mean)
                || spec
                    .supported_dimensions
                    .iter()
                    .any(|size| ![128, 256, 512, 768].contains(size)))
        {
            return Err(anyhow!(
                "EmbeddingGemma 2 uses mean pooling and supports 128, 256, 512, or 768 dimensions"
            ));
        }
        let store = FlowLikeState::bit_store(&state).await?;
        let FlowLikeStore::Local(store) = store else {
            return Err(anyhow!(
                "Local multimodal embedding requires a filesystem-backed Bit store"
            ));
        };
        let pack = bit.pack(state.clone()).await?;
        let pinned = spec
            .artifacts
            .values()
            .all(|artifact| artifact.source.is_some());
        // Pinned cached files are checked against their digests while materializing the bundle.
        if !pinned || !pack.is_installed(state.clone()).await? {
            ensure_local_weights(&pack, &state, &bit.id, "multimodal embedding model").await?;
        }
        Self::from_installed_pack(bit, spec, &pack, &store).await
    }

    async fn from_installed_pack(
        bit: &Bit,
        spec: EmbeddingSpec,
        pack: &BitPack,
        store: &Arc<LocalObjectStore>,
    ) -> Result<Arc<Self>> {
        let sources = resolve_artifacts(&spec, pack)?;
        let mut fingerprint = blake3::Hasher::new();
        fingerprint.update(&serde_json::to_vec(&spec)?);
        let mut files = Vec::new();
        for (role, source) in sources {
            let artifact = &spec.artifacts[&role];
            let path = source
                .to_path(store)
                .ok_or(anyhow!("Embedding artifact {role} has no local path"))?;
            let expected_hash = artifact.source.as_ref().map(|source| source.hash.clone());
            files.push((role, artifact.path.clone(), path, expected_hash));
        }
        let bundle = Arc::new(
            tempfile::Builder::new()
                .prefix("flow-like-embedding-")
                .tempdir()?,
        );
        let directory = bundle.path().to_path_buf();
        let pipeline_fingerprint =
            flow_like_types::tokio::task::spawn_blocking(move || -> Result<String> {
                use std::io::Read;
                let mut buffer = vec![0u8; 1024 * 1024];
                for (role, relative, source, expected_hash) in files {
                    let mut file = std::fs::File::open(&source)?;
                    let mut content = blake3::Hasher::new();
                    loop {
                        let count = file.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        content.update(&buffer[..count]);
                    }
                    let hash = content.finalize();
                    if expected_hash.is_some_and(|expected| {
                        !hash.to_hex().as_str().eq_ignore_ascii_case(&expected)
                    }) {
                        return Err(anyhow!(
                            "Embedding artifact {role} does not match its pinned content hash"
                        ));
                    }
                    fingerprint.update(role.as_bytes());
                    fingerprint.update(hash.as_bytes());
                    let target = directory.join(relative);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    if std::fs::hard_link(&source, &target).is_err() {
                        std::fs::copy(&source, &target)?;
                    }
                }
                Ok(fingerprint.finalize().to_hex().to_string())
            })
            .await??;
        Self::load_bundle(bit, spec, bundle, pipeline_fingerprint).await
    }

    async fn load_bundle(
        bit: &Bit,
        spec: EmbeddingSpec,
        bundle: Arc<tempfile::TempDir>,
        pipeline_fingerprint: String,
    ) -> Result<Arc<Self>> {
        let path_for = |role: &str| {
            spec.artifacts
                .get(role)
                .map(|asset| std::path::PathBuf::from(&asset.path))
        };
        let mut max_tokens = if cfg!(any(
            target_os = "ios",
            target_os = "tvos",
            target_os = "android"
        )) {
            spec.max_tokens.min(2048)
        } else {
            spec.max_tokens.min(8192)
        };
        let directory = bundle.path().to_path_buf();
        let (model, tokenizer, modalities, purposes, splitter_capacity) =
            if spec.adapter == "embedding_gemma2" {
                if !spec.task_profiles.is_empty() {
                    return Err(anyhow!("EmbeddingGemma 2 defines its own task profiles"));
                }
                let options = Gemma2Options {
                    model_file: path_for("backbone")
                        .ok_or(anyhow!("EmbeddingGemma 2 needs a backbone artifact"))?,
                    vision_file: path_for("vision_encoder"),
                    audio_file: path_for("audio_encoder"),
                    tokenizer_file: path_for("tokenizer")
                        .ok_or(anyhow!("EmbeddingGemma 2 needs a tokenizer artifact"))?,
                    processor_file: path_for("processor").ok_or(anyhow!(
                        "EmbeddingGemma 2 needs a processor configuration artifact"
                    ))?,
                    max_tokens,
                    ..Default::default()
                };
                let vision = options.vision_file.is_some();
                let audio = options.audio_file.is_some();
                let model = flow_like_types::tokio::task::spawn_blocking(move || {
                    Gemma2Embedding::load(&directory, options)
                })
                .await??;
                let tokenizer = model.tokenizer().clone();
                let mut modalities = vec![EmbeddingModality::Text];
                if vision {
                    modalities.extend([EmbeddingModality::Image, EmbeddingModality::Video]);
                }
                if audio {
                    modalities.push(EmbeddingModality::Audio);
                }
                // Reserve task-prefix, title-label and special-token overhead for old splitter nodes.
                (
                    LocalEmbeddingBackend::Gemma2 {
                        preprocessor: model.preprocessor(),
                        model: Mutex::new(model),
                    },
                    tokenizer,
                    modalities,
                    vec![
                        EmbeddingPurpose::Query,
                        EmbeddingPurpose::Document,
                        EmbeddingPurpose::Similarity,
                        EmbeddingPurpose::Classification,
                        EmbeddingPurpose::Clustering,
                        EmbeddingPurpose::CodeQuery,
                    ],
                    max_tokens.saturating_sub(32).max(1),
                )
            } else {
                let load_spec = spec.clone();
                let (text, image) = flow_like_types::tokio::task::spawn_blocking(move || {
                    load_dual_encoder(&directory, &load_spec, max_tokens)
                })
                .await??;
                max_tokens = text
                    .tokenizer()
                    .get_truncation()
                    .map_or(max_tokens, |config| config.max_length);
                let tokenizer = text.tokenizer().clone();
                let overhead = spec
                    .task_profiles
                    .values()
                    .map(|profile| {
                        tokenizer
                            .encode(format!("{}{}", profile.prefix, profile.suffix), true)
                            .map(|encoding| encoding.len())
                            .map_err(|error| anyhow!(error.to_string()))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .max()
                    .unwrap_or(2);
                let mut modalities = vec![EmbeddingModality::Text];
                if image.is_some() {
                    modalities.push(EmbeddingModality::Image);
                }
                let purposes = if spec.task_profiles.is_empty() {
                    vec![EmbeddingPurpose::Query, EmbeddingPurpose::Document]
                } else {
                    spec.task_profiles.keys().copied().collect()
                };
                (
                    LocalEmbeddingBackend::DualEncoder {
                        text_preprocessor: text.preprocessor(),
                        text: Mutex::new(text),
                        image: image.map(|image| {
                            let preprocessor = image.preprocessor().clone();
                            (Mutex::new(image), preprocessor)
                        }),
                        profiles: spec.task_profiles.clone(),
                        max_tokens,
                    },
                    tokenizer,
                    modalities,
                    purposes,
                    max_tokens.saturating_sub(overhead).max(1),
                )
            };
        let sizer = TokenizerSizer::new(tokenizer);
        let joint_combinations = if spec.adapter == "embedding_gemma2" {
            modality_combinations(&modalities)
        } else {
            vec![]
        };
        let mut supported_dimensions =
            if spec.supported_dimensions.is_empty() && spec.adapter == "embedding_gemma2" {
                vec![128, 256, 512, 768]
            } else {
                spec.supported_dimensions
            };
        supported_dimensions.push(spec.dimensions);
        supported_dimensions.sort_unstable();
        supported_dimensions.dedup();
        let descriptor = EmbeddingDescriptor {
            model_id: bit.id.clone(),
            adapter: spec.adapter,
            modalities,
            joint_combinations,
            purposes,
            space: EmbeddingSpace {
                id: spec.space_id,
                dimensions: spec.dimensions,
                normalized: true,
                metric: EmbeddingMetric::Cosine,
            },
            supported_dimensions,
            limits: EmbeddingLimits {
                max_tokens,
                max_batch_items: Some(2048),
                ..Default::default()
            },
            pipeline_fingerprint,
        };
        Ok(Arc::new(Self {
            descriptor,
            model: Arc::new(model),
            preparation_slots: Arc::new(Semaphore::new(PREPARATION_SLOTS)),
            sizer,
            splitter_capacity,
            _bundle: bundle,
        }))
    }
}

fn load_dual_encoder(
    directory: &Path,
    spec: &EmbeddingSpec,
    max_tokens: usize,
) -> Result<(NativeTextEmbedding, Option<NativeImageEmbedding>)> {
    let path = |role: &str| {
        spec.artifacts
            .get(role)
            .map(|asset| directory.join(&asset.path))
            .ok_or(anyhow!(
                "Embedding adapter {} requires artifact {role}",
                spec.adapter
            ))
    };
    let files = TokenizerFiles {
        tokenizer_file: std::fs::read(path("tokenizer")?)?,
        config_file: std::fs::read(path("config")?)?,
        tokenizer_config_file: std::fs::read(path("tokenizer_config")?)?,
        special_tokens_map_file: match spec.artifacts.get("special_tokens") {
            Some(asset) => std::fs::read(directory.join(&asset.path))?,
            None => vec![],
        },
    };
    let pooling = match spec.pooling {
        EmbeddingPooling::Mean => Pooling::Mean,
        EmbeddingPooling::Cls => Pooling::Cls,
        EmbeddingPooling::LastToken => Pooling::LastToken,
    };
    let text = NativeTextEmbedding::new_from_file(
        path("backbone")?,
        files,
        max_tokens,
        pooling,
        SessionOptions::default(),
    )?;
    if text
        .output_dimensions()
        .is_some_and(|dimensions| dimensions != spec.dimensions)
    {
        return Err(anyhow!(
            "Text graph dimensions do not match the embedding recipe"
        ));
    }
    let image = if spec.adapter == "clip" {
        let model = NativeImageEmbedding::new_from_file(
            path("vision_encoder")?,
            &std::fs::read(path("processor")?)?,
            SessionOptions::default(),
        )?;
        if model
            .output_dimensions()
            .is_some_and(|dimensions| dimensions != spec.dimensions)
        {
            return Err(anyhow!(
                "Image graph dimensions do not match its paired text space"
            ));
        }
        Some(model)
    } else {
        None
    };
    Ok((text, image))
}

fn resolve_artifacts<'a>(
    spec: &EmbeddingSpec,
    pack: &'a BitPack,
) -> Result<Vec<(String, &'a Bit)>> {
    let mut result = Vec::new();
    for (role, asset) in &spec.artifacts {
        let matches = pack
            .bits
            .iter()
            .filter(|bit| asset.bit == bit.id || asset.bit == format!("{}:{}", bit.hub, bit.id))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(anyhow!(
                "Embedding artifact role {role} must resolve to exactly one Bit dependency"
            ));
        }
        result.push((role.clone(), matches[0]));
    }
    Ok(result)
}

fn modality_combinations(modalities: &[EmbeddingModality]) -> Vec<Vec<EmbeddingModality>> {
    (1usize..(1 << modalities.len()))
        .filter(|mask| mask.count_ones() > 1)
        .map(|mask| {
            modalities
                .iter()
                .enumerate()
                .filter_map(|(index, modality)| ((mask & (1 << index)) != 0).then_some(*modality))
                .collect()
        })
        .collect()
}

#[async_trait]
impl EmbeddingModel for LocalMultimodalEmbeddingModel {
    fn descriptor(&self) -> &EmbeddingDescriptor {
        &self.descriptor
    }

    async fn embed(
        &self,
        request: EmbeddingRequest,
    ) -> std::result::Result<EmbeddingBatch, EmbeddingError> {
        self.descriptor.validate(&request)?;
        let model = self.model.clone();
        let permit = self
            .preparation_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| EmbeddingError::Backend(error.into()))?;
        let (request, vectors) = flow_like_types::tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let vectors = model.embed(&request.items, request.purpose)?;
            Ok::<_, flow_like_types::Error>((request, vectors))
        })
        .await
        .map_err(|error| EmbeddingError::Backend(error.into()))?
        .map_err(EmbeddingError::Backend)?;
        self.descriptor.finish(&request, vectors)
    }
}

impl Cacheable for LocalMultimodalEmbeddingModel {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl EmbeddingModelLogic for LocalMultimodalEmbeddingModel {
    fn context_tokens(&self) -> Option<usize> {
        Some(self.descriptor.limits.max_tokens)
    }

    fn output_dimensions(&self) -> Option<usize> {
        Some(self.descriptor.space.dimensions)
    }
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        let capacity = capacity
            .unwrap_or(self.splitter_capacity)
            .min(self.splitter_capacity)
            .max(1);
        let config = ChunkConfig::new(capacity)
            .with_sizer(self.sizer.clone())
            .with_overlap(overlap.unwrap_or(20).min(capacity.saturating_sub(1)))?;
        let config_md = ChunkConfig::new(capacity)
            .with_sizer(self.sizer.clone())
            .with_overlap(overlap.unwrap_or(20).min(capacity.saturating_sub(1)))?;
        Ok((
            GeneralTextSplitter::TextTokenizer(Arc::new(TextSplitter::new(config))),
            GeneralTextSplitter::MarkdownTokenizer(Arc::new(MarkdownSplitter::new(config_md))),
        ))
    }

    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .embed(EmbeddingRequest::texts(
                texts.iter().cloned(),
                EmbeddingPurpose::Query,
            ))
            .await?
            .embeddings)
    }
    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .embed(EmbeddingRequest::texts(
                texts.iter().cloned(),
                EmbeddingPurpose::Document,
            ))
            .await?
            .embeddings)
    }
    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}

#[async_trait]
impl ImageEmbeddingModelLogic for LocalMultimodalEmbeddingModel {
    fn context_tokens(&self) -> Option<usize> {
        Some(self.descriptor.limits.max_tokens)
    }

    fn output_dimensions(&self) -> Option<usize> {
        Some(self.descriptor.space.dimensions)
    }
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        EmbeddingModelLogic::get_splitter(self, capacity, overlap).await
    }
    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        EmbeddingModelLogic::text_embed_query(self, texts).await
    }
    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        EmbeddingModelLogic::text_embed_document(self, texts).await
    }
    async fn image_embed(&self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>> {
        Ok(self
            .embed(EmbeddingRequest {
                items: images.into_iter().map(EmbeddingInput::image).collect(),
                purpose: EmbeddingPurpose::Document,
                options: EmbeddingOptions::default(),
            })
            .await?
            .embeddings)
    }
    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_roles_do_not_depend_on_pack_order_or_bit_type() {
        let spec: EmbeddingSpec = serde_json::from_value(serde_json::json!({
            "schema_version":1,"adapter":"embedding_gemma2","space_id":"test","dimensions":768,"max_tokens":8192,
            "artifacts":{"backbone":{"bit":"hub:backbone","path":"onnx/model.onnx"},"vision_encoder":{"bit":"vision","path":"onnx/vision.onnx"}}
        })).unwrap();
        let pack = BitPack {
            bits: vec![
                Bit {
                    id: "vision".into(),
                    ..Bit::default()
                },
                Bit {
                    id: "backbone".into(),
                    hub: "hub".into(),
                    ..Bit::default()
                },
            ],
        };
        let resolved = resolve_artifacts(&spec, &pack).unwrap();
        assert_eq!(resolved[0].1.id, "backbone");
        let mut missing = pack.clone();
        missing.bits.pop();
        assert!(resolve_artifacts(&spec, &missing).is_err());
    }

    #[test]
    fn absent_recipe_retains_legacy_path_but_unknown_recipe_version_fails() {
        assert!(embedding_spec(&Bit::default()).unwrap().is_none());
        let bit = Bit {
            parameters: serde_json::json!({"embedding":{"schema_version":99,"adapter":"embedding_gemma2","space_id":"test","dimensions":768,"max_tokens":8192,"artifacts":{}}}),
            ..Bit::default()
        };
        assert!(embedding_spec(&bit).is_err());
    }

    fn install_recipe(
        adapter: &str,
        dimensions: usize,
        max_tokens: usize,
        assets: Vec<(String, std::path::PathBuf, String)>,
    ) -> Result<(
        tempfile::TempDir,
        Arc<LocalObjectStore>,
        Bit,
        BitPack,
        EmbeddingSpec,
    )> {
        let cache = tempfile::tempdir()?;
        let store = Arc::new(LocalObjectStore::new(cache.path().to_path_buf())?);
        let mut artifacts = serde_json::Map::new();
        let mut bits = Vec::new();
        for (index, (role, source, path)) in assets.into_iter().enumerate() {
            let id = format!("fixture-{index}");
            let bit = Bit {
                id: id.clone(),
                hash: id.clone(),
                file_name: Some(format!("asset-{index}")),
                ..Bit::default()
            };
            let target = bit.to_path(&store).unwrap();
            std::fs::create_dir_all(target.parent().unwrap())?;
            if std::fs::hard_link(&source, &target).is_err() {
                std::fs::copy(source, target)?;
            }
            artifacts.insert(role, serde_json::json!({"bit":id,"path":path}));
            bits.push(bit);
        }
        bits.reverse();
        let recipe = serde_json::json!({"schema_version":1,"adapter":adapter,"space_id":"fixture-space","dimensions":dimensions,"max_tokens":max_tokens,"artifacts":artifacts,"pooling":"cls"});
        let bit = Bit {
            id: "fixture-root".into(),
            parameters: serde_json::json!({"embedding":recipe}),
            ..Bit::default()
        };
        let spec = embedding_spec(&bit)?.unwrap();
        Ok((cache, store, bit, BitPack { bits }, spec))
    }

    fn text_assets(directory: &Path) -> Vec<(String, std::path::PathBuf, String)> {
        [
            ("backbone", "model.onnx"),
            ("tokenizer", "tokenizer.json"),
            ("config", "config.json"),
            ("tokenizer_config", "tokenizer_config.json"),
            ("special_tokens", "special_tokens_map.json"),
        ]
        .into_iter()
        .map(|(role, file)| (role.into(), directory.join(file), file.into()))
        .collect()
    }

    #[flow_like_types::tokio::test]
    #[ignore = "Requires published assets in FLOW_LIKE_EMBEDDING_MODELS"]
    async fn installed_text_and_clip_recipes_preserve_vector_space_and_legacy_entry_points()
    -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("FLOW_LIKE_EMBEDDING_MODELS")?);
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../model-provider/tests/fixtures/embedding_parity");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixtures.join("manifest.json"))?)?;
        let input = manifest["inputs"][1].as_str().unwrap().to_owned();
        for (adapter, id, dimensions, max_tokens) in [
            (
                "sentence_transformer",
                "rkhhottkdbbybjsplrxfmnhk",
                384,
                8192,
            ),
            ("clip", "f6neh9w69tvfwqjuvde5yhsq", 512, 50),
        ] {
            let mut assets = text_assets(&root.join(id));
            if adapter == "clip" {
                let vision = root.join("k10qlw21o4ct379j3lpy7fo1");
                assets.push((
                    "vision_encoder".into(),
                    vision.join("model.onnx"),
                    "vision.onnx".into(),
                ));
                assets.push((
                    "processor".into(),
                    vision.join("preprocessor_config.json"),
                    "preprocessor_config.json".into(),
                ));
            }
            let (_cache, store, bit, pack, spec) =
                install_recipe(adapter, dimensions, max_tokens, assets)?;
            let model =
                LocalMultimodalEmbeddingModel::from_installed_pack(&bit, spec, &pack, &store)
                    .await?;
            assert_eq!(model.descriptor().supported_dimensions, vec![dimensions]);
            let result = model
                .embed(EmbeddingRequest::texts(
                    [input.clone()],
                    EmbeddingPurpose::Query,
                ))
                .await?;
            let vectors = std::fs::read(fixtures.join(format!("{id}.f32")))?;
            let reference = manifest["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model["bit"]["id"].as_str() == Some(id))
                .unwrap();
            // Dynamic quantization depends on padding, so compare identical single-item batches.
            let case = reference["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["name"] == "raw_batch_1")
                .unwrap();
            let offset = case["offset_floats"].as_u64().unwrap() as usize;
            let expected = vectors
                .chunks_exact(4)
                .skip(offset + dimensions)
                .take(dimensions)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                .collect::<Vec<_>>();
            assert!(
                cosine_distance(&result.embeddings[0], &expected) < 1e-5,
                "{adapter}"
            );
            let legacy =
                EmbeddingModelLogic::text_embed_query(model.as_ref(), &vec![input.clone()]).await?;
            assert_eq!(legacy, result.embeddings);
            assert_eq!(
                EmbeddingModelLogic::output_dimensions(model.as_ref()),
                Some(dimensions)
            );
            assert!(!result.provenance.pipeline_fingerprint.is_empty());
            let LocalEmbeddingBackend::DualEncoder { text, .. } = model.model.as_ref() else {
                unreachable!()
            };
            let session = text.lock().await;
            let overflow = flow_like_types::tokio::time::timeout(
                std::time::Duration::from_secs(5),
                model.embed(EmbeddingRequest::texts(
                    ["too long ".repeat(max_tokens)],
                    EmbeddingPurpose::Query,
                )),
            )
            .await;
            drop(session);
            assert!(
                overflow
                    .expect("context validation waited for the inference lock")
                    .is_err()
            );
            let requests =
                ["hello", "two short words", "", "different padding length"].map(|value| {
                    EmbeddingRequest::texts(
                        [value.to_owned(), "a document".into()],
                        EmbeddingPurpose::Query,
                    )
                });
            let mut expected = Vec::new();
            for request in &requests {
                expected.push(model.embed(request.clone()).await?);
            }
            let concurrent = futures::future::try_join_all(
                requests.into_iter().map(|request| model.embed(request)),
            )
            .await?;
            for (actual, expected) in concurrent.iter().zip(&expected) {
                for (actual, expected) in actual.embeddings.iter().zip(&expected.embeddings) {
                    assert!(
                        cosine_distance(actual, expected) < 1e-5,
                        "concurrent {adapter}"
                    );
                }
            }
            assert_eq!(
                model.preparation_slots.available_permits(),
                PREPARATION_SLOTS
            );
            if adapter == "clip" {
                let image = EmbeddingInput::image(DynamicImage::new_rgb8(32, 24));
                let request = EmbeddingRequest {
                    items: vec![image.clone(), EmbeddingInput::text(input.clone())],
                    purpose: EmbeddingPurpose::Document,
                    options: EmbeddingOptions::default(),
                };
                let batch = model.embed(request).await?;
                assert_eq!(batch.embeddings.len(), 2);
                assert!(cosine_distance(&batch.embeddings[1], &result.embeddings[0]) < 1e-5);
                let joint = EmbeddingInput {
                    parts: vec![EmbeddingPart::Text(input.clone()), image.parts[0].clone()],
                    title: None,
                };
                assert!(matches!(
                    model
                        .embed(EmbeddingRequest {
                            items: vec![joint],
                            purpose: EmbeddingPurpose::Query,
                            options: EmbeddingOptions::default()
                        })
                        .await,
                    Err(EmbeddingError::UnsupportedCombination(_))
                ));
            }
        }
        Ok(())
    }

    #[flow_like_types::tokio::test]
    #[ignore = "Requires pinned q4 assets in FLOW_LIKE_GEMMA2_MODEL"]
    async fn installed_gemma2_recipe_runs_all_modalities_and_reduced_output() -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("FLOW_LIKE_GEMMA2_MODEL")?);
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../model-provider/tests/fixtures/gemma2/bit.json");
        let bit: Bit = serde_json::from_slice(&std::fs::read(fixture)?)?;
        let spec = embedding_spec(&bit)?.unwrap();
        let cache = tempfile::tempdir()?;
        let store = Arc::new(LocalObjectStore::new(cache.path().to_path_buf())?);
        for dependency in bit.inline_embedding_asset_bits()? {
            let artifact = spec
                .artifacts
                .values()
                .find(|asset| asset.bit == dependency.id)
                .unwrap();
            let source = root.join(&artifact.path);
            let target = dependency.to_path(&store).unwrap();
            std::fs::create_dir_all(target.parent().unwrap())?;
            if std::fs::hard_link(&source, &target).is_err() {
                std::fs::copy(source, target)?;
            }
        }
        let config = crate::state::FlowLikeConfig::with_default_store(FlowLikeStore::Local(store));
        let state = Arc::new(FlowLikeState::new(
            config,
            crate::utils::http::HTTPClient::new_without_refetch(),
        ));
        let factory = crate::models::embedding_factory::EmbeddingFactory::new();
        let model = factory.build(&bit, state.clone(), None, None).await?;
        let cached = factory.build(&bit, state.clone(), None, None).await?;
        assert!(Arc::ptr_eq(&model, &cached));
        let image = Arc::new(DynamicImage::new_rgb8(32, 24));
        let audio = AudioInput {
            samples: (0..16_000)
                .map(|index| (index as f32 * 0.1).sin() * 0.1)
                .collect::<Vec<_>>()
                .into(),
            sample_rate: 16_000,
            channels: 1,
        };
        let parts = vec![
            EmbeddingPart::Text("An image with a sound".into()),
            EmbeddingPart::Image(image.clone()),
            EmbeddingPart::Audio(audio),
            EmbeddingPart::Video(VideoInput {
                frames: vec![VideoFrame {
                    timestamp_ms: 0,
                    image,
                }],
                duration_ms: 1000,
                audio: None,
            }),
        ];
        let mut items = parts
            .iter()
            .map(|part| EmbeddingInput {
                parts: vec![part.clone()],
                title: None,
            })
            .collect::<Vec<_>>();
        let requests = [128, 256, 512, 768]
            .into_iter()
            .zip(&items)
            .map(|(dimensions, item)| EmbeddingRequest {
                items: vec![item.clone()],
                purpose: EmbeddingPurpose::Document,
                options: EmbeddingOptions {
                    dimensions: Some(dimensions),
                },
            })
            .collect::<Vec<_>>();
        let mut expected = Vec::new();
        for request in &requests {
            expected.push(model.embed(request.clone()).await?);
        }
        let concurrent =
            futures::future::try_join_all(requests.into_iter().map(|request| model.embed(request)))
                .await?;
        for (actual, expected) in concurrent.iter().zip(&expected) {
            assert_eq!(actual.space, expected.space);
            assert!(cosine_distance(&actual.embeddings[0], &expected.embeddings[0]) < 1e-5);
        }
        items.push(EmbeddingInput {
            parts,
            title: Some("A multimodal document".into()),
        });
        let result = model
            .embed(EmbeddingRequest {
                items,
                purpose: EmbeddingPurpose::Document,
                options: EmbeddingOptions {
                    dimensions: Some(128),
                },
            })
            .await?;
        assert_eq!(result.embeddings.len(), 5);
        assert_eq!(result.space.dimensions, 128);
        assert_eq!(result.usage.audio_duration_ms, 2000);
        assert_eq!(result.usage.video_frames, 2);
        for vector in result.embeddings {
            assert_eq!(vector.len(), 128);
        }
        let text = factory.build_text(&bit, state).await?;
        let legacy = text.text_embed_document(&vec!["A document".into()]).await?;
        assert_eq!(legacy[0].len(), 768);
        Ok(())
    }

    fn cosine_distance(left: &[f32], right: &[f32]) -> f64 {
        let dot: f64 = left
            .iter()
            .zip(right)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum();
        let norm = |values: &[f32]| {
            values
                .iter()
                .map(|&value| f64::from(value).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        1.0 - dot / (norm(left) * norm(right))
    }
}
