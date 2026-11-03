use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::{KnowledgeEmbedder, ModelEmbedder, ModelRole};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_runtime::bit::Bit;
use flow_like_runtime::models::llm::ModelUsageContext;
use flow_like_runtime::state::FlowLikeState;
use flow_like_types::{Result, anyhow};

/// Where a host gets the chat model and the embedder of a bot.
#[async_trait]
pub trait ModelSource: Send + Sync {
    async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>>;
    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>>;
}

/// Models built from Bits through a state's factories. `token` and `usage` route hosted and
/// private models through the API proxy and bill the turn.
pub struct StateModels {
    pub state: Arc<FlowLikeState>,
    pub chat: Option<Bit>,
    pub embedding: Option<Bit>,
    pub token: Option<String>,
    pub usage: ModelUsageContext,
}

#[async_trait]
impl ModelSource for StateModels {
    async fn model(&self, _role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        let bit = self
            .chat
            .as_ref()
            .ok_or_else(|| anyhow!("This turn has no chat model for the tool to use"))?;
        self.state
            .model_factory
            .build(
                bit,
                self.state.clone(),
                self.token.clone(),
                Some(self.usage.clone()),
            )
            .await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        let Some(bit) = &self.embedding else {
            return Ok(None);
        };
        let logic = self
            .state
            .embedding_factory
            .build_text_routed(
                bit,
                self.state.clone(),
                self.token.clone(),
                Some(self.usage.clone()),
            )
            .await?;
        let embedder = ModelEmbedder::new(bit.id.clone(), logic, Some(512), Some(64)).await?;
        Ok(Some(Arc::new(embedder)))
    }
}
