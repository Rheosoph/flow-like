//! Embedding handles shared by native nodes and WASM host functions.

extern crate flow_like_runtime as flow_like;

use flow_like::bit::BitTypes;
use flow_like_model_provider::{
    embedding::EmbeddingModelLogic, image_embedding::ImageEmbeddingModelLogic,
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

pub struct CachedEmbeddingModelObject {
    pub text_model: Option<Arc<dyn EmbeddingModelLogic>>,
    pub image_model: Option<Arc<dyn ImageEmbeddingModelLogic>>,
}

impl Cacheable for CachedEmbeddingModelObject {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
