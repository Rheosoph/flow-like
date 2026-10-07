//! Embedding handles shared by native nodes and WASM host functions.

extern crate flow_like_runtime as flow_like;

use flow_like::{bit::BitTypes, flow::execution::context::ExecutionContext};
use flow_like_model_provider::{
    embedding::{EmbeddingModelLogic, interface::EmbeddingModel},
    image_embedding::ImageEmbeddingModelLogic,
};
use flow_like_types::{
    Cacheable, JsonSchema,
    json::{Deserialize, Serialize},
};
use std::{any::Any, sync::Arc};

#[derive(Clone, Serialize, Deserialize, JsonSchema, Debug)]
pub struct CachedEmbeddingModel {
    pub cache_key: String,
    pub model_type: BitTypes,
}

impl CachedEmbeddingModel {
    pub async fn resolve_model(
        &self,
        context: &ExecutionContext,
    ) -> flow_like_types::Result<Arc<dyn EmbeddingModel>> {
        let cached = context
            .get_cache(&self.cache_key)
            .await
            .ok_or_else(|| flow_like_types::anyhow!("Embedding model not found in cache"))?;
        cached
            .as_any()
            .downcast_ref::<CachedEmbeddingModelObject>()
            .ok_or_else(|| flow_like_types::anyhow!("Cached object is not an embedding model"))?
            .model
            .clone()
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Reload the model with Load Embedding Model to embed structured content"
                )
            })
    }
}

pub struct CachedEmbeddingModelObject {
    pub text_model: Option<Arc<dyn EmbeddingModelLogic>>,
    pub image_model: Option<Arc<dyn ImageEmbeddingModelLogic>>,
    pub model: Option<Arc<dyn EmbeddingModel>>,
}

impl Cacheable for CachedEmbeddingModelObject {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
