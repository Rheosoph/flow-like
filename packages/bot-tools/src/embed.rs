use std::sync::Arc;

use async_trait::async_trait;
use flow_like_model_provider::embedding::{EmbeddingModelLogic, GeneralTextSplitter};
use flow_like_types::Result;

/// The bot's embedding model with its own text splitter.
#[async_trait]
pub trait KnowledgeEmbedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn split(&self, markdown: &str) -> Result<Vec<String>>;
    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

/// Adapts an [`EmbeddingModelLogic`] (the model provider's embedding models) to the tools.
pub struct ModelEmbedder {
    model_id: String,
    logic: Arc<dyn EmbeddingModelLogic>,
    splitter: GeneralTextSplitter,
}

impl ModelEmbedder {
    /// `capacity` is the chunk size in the model's tokens (512 suits most models) and `overlap`
    /// the tokens two neighbouring chunks share.
    pub async fn new(
        model_id: impl Into<String>,
        logic: Arc<dyn EmbeddingModelLogic>,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> Result<Self> {
        let (_, markdown) = logic.get_splitter(capacity, overlap).await?;
        Ok(Self {
            model_id: model_id.into(),
            logic,
            splitter: markdown,
        })
    }
}

#[async_trait]
impl KnowledgeEmbedder for ModelEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn split(&self, markdown: &str) -> Result<Vec<String>> {
        self.splitter.chunks(markdown)
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.logic.text_embed_document(&texts.to_vec()).await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let mut vectors = self.logic.text_embed_query(&vec![text.to_string()]).await?;
        vectors.pop().ok_or_else(|| {
            flow_like_types::anyhow!(
                "Embedding model '{}' returned no vector for the query",
                self.model_id
            )
        })
    }
}
