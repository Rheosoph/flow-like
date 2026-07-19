#![cfg(feature = "local-ml")]
use crate::{
    bit::{Bit, BitTypes},
    models::{embedding_factory::EmbeddingFactory, local_utils::ensure_local_weights},
    state::FlowLikeState,
};
use flow_like_model_provider::fastembed::{
    self, ImageEmbedding, ImageInitOptionsUserDefined, UserDefinedImageEmbeddingModel,
};
use flow_like_model_provider::{
    embedding::{EmbeddingModelLogic, GeneralTextSplitter},
    image_embedding::ImageEmbeddingModelLogic,
    ml::ort_runtime::{ensure_ort_initialized, session_execution_providers},
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{Cacheable, Result, async_trait, sync::Mutex};
use image::DynamicImage;
use std::{any::Any, sync::Arc};

#[derive(Clone)]
pub struct LocalImageEmbeddingModel {
    pub bit: Arc<Bit>,
    image_embedding_model: Arc<Mutex<fastembed::ImageEmbedding>>,
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
        factory: &mut EmbeddingFactory,
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
        let loaded_model = std::fs::read(model_path)?;

        let preprocessor_path = preprocessor_bit
            .to_path(&bit_store)
            .ok_or(flow_like_types::anyhow!("No model path"))?;
        let loaded_preprocessor = std::fs::read(preprocessor_path)?;

        let user_embedding_model =
            UserDefinedImageEmbeddingModel::new(loaded_model, loaded_preprocessor);
        ensure_ort_initialized().map_err(|error| {
            flow_like_types::anyhow!("Failed to configure ONNX Runtime: {error}")
        })?;
        let init_options = ImageInitOptionsUserDefined::new().with_execution_providers(
            session_execution_providers(true).map_err(|error| {
                flow_like_types::anyhow!("Failed to select ONNX providers: {error}")
            })?,
        );

        let loaded_model = ImageEmbedding::try_new_from_user_defined(
            user_embedding_model.clone(),
            init_options.clone(),
        )?;

        let default_return_model = LocalImageEmbeddingModel {
            bit,
            image_embedding_model: Arc::new(Mutex::new(loaded_model)),
            text_model,
        };

        Ok(Arc::new(default_return_model))
    }
}

#[async_trait]
impl ImageEmbeddingModelLogic for LocalImageEmbeddingModel {
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
        let model = self.image_embedding_model.clone();
        let embeddings = flow_like_types::tokio::task::spawn_blocking(move || {
            model.blocking_lock().embed_images(images)
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
