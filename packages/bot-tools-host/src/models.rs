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

impl StateModels {
    /// The same state, token and usage with `chat` as the chat model and no embedder.
    pub fn answering_with(&self, chat: Bit) -> StateModels {
        StateModels {
            state: self.state.clone(),
            chat: Some(chat),
            embedding: None,
            token: self.token.clone(),
            usage: self.usage.clone(),
        }
    }
}

/// Sends [`ModelRole::Vision`] to the bot's vision model and every other role to `base`. A
/// vision model that cannot be built leaves the read to `base`.
pub struct VisionRouted {
    base: Arc<dyn ModelSource>,
    vision: Option<Arc<dyn ModelSource>>,
}

impl VisionRouted {
    pub fn new(base: Arc<dyn ModelSource>, vision: Option<Arc<dyn ModelSource>>) -> Self {
        Self { base, vision }
    }
}

#[async_trait]
impl ModelSource for VisionRouted {
    async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        if role == ModelRole::Vision
            && let Some(vision) = &self.vision
            && let Ok(model) = vision.model(ModelRole::Default).await
        {
            return Ok(model);
        }
        self.base.model(role).await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        self.base.embedder().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Recording {
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ModelSource for Recording {
        async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:{role:?}", self.name));
            Err(anyhow!("{}", self.name))
        }

        async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
            Ok(None)
        }
    }

    fn routed(vision: bool) -> (VisionRouted, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let source = |name| {
            Arc::new(Recording {
                name,
                calls: calls.clone(),
            }) as Arc<dyn ModelSource>
        };
        (
            VisionRouted::new(source("base"), vision.then(|| source("vision"))),
            calls,
        )
    }

    #[tokio::test]
    async fn a_vision_read_asks_the_vision_model_first_and_falls_back_to_the_base() {
        let (models, calls) = routed(true);
        let error = models.model(ModelRole::Vision).await.err().unwrap();
        assert_eq!(error.to_string(), "base");
        assert_eq!(*calls.lock().unwrap(), ["vision:Default", "base:Vision"]);
    }

    #[tokio::test]
    async fn other_roles_and_a_bot_without_a_vision_model_use_the_base() {
        let (models, calls) = routed(true);
        let _ = models.model(ModelRole::Default).await;
        let (plain, plain_calls) = routed(false);
        let _ = plain.model(ModelRole::Vision).await;
        assert_eq!(*calls.lock().unwrap(), ["base:Default"]);
        assert_eq!(*plain_calls.lock().unwrap(), ["base:Vision"]);
    }
}
