use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use image::DynamicImage;
use std::sync::Arc;

use crate::embedding::GeneralTextSplitter;

#[async_trait]
// `&Vec<String>` is baked into implementors across other crates; widening it to `&[String]` is a
// cross-crate signature change, not a local readability fix.
#[allow(clippy::ptr_arg)]
pub trait ImageEmbeddingModelLogic: Send + Sync + Cacheable + 'static {
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> anyhow::Result<(GeneralTextSplitter, GeneralTextSplitter)>;
    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>>;
    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>>;
    async fn image_embed(&self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>>;
    fn as_cacheable(&self) -> Arc<dyn Cacheable>;
}
