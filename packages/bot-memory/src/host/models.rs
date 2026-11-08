//! The engine's two model plugs over the model provider's traits, and swappable handles so one
//! engine per bot outlives the per-turn tokens its models are built with.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use flow_like_model_provider::embedding::EmbeddingModelLogic;
use flow_like_model_provider::history::{History, HistoryMessage, ResponseFormat, Role};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_model_provider::systemone::{
    SystemOneModelLogic, SystemOneRequest, SystemOneResponse,
};
use flow_like_types::{Result, anyhow};
use serde_json::json;

use crate::llm::{Embedder, LlmRequest, LlmTask, MemoryLlm};

/// The bot's embedding model: `model_id` is the bot's `embedding_model` bit id.
pub struct ModelEmbedder {
    model_id: String,
    logic: Arc<dyn EmbeddingModelLogic>,
}

impl ModelEmbedder {
    pub fn new(model_id: impl Into<String>, logic: Arc<dyn EmbeddingModelLogic>) -> Self {
        Self {
            model_id: model_id.into(),
            logic,
        }
    }
}

#[async_trait]
impl Embedder for ModelEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.logic.text_embed_document(&texts.to_vec()).await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let mut vectors = self.logic.text_embed_query(&vec![text.to_string()]).await?;
        vectors.pop().ok_or_else(|| {
            anyhow!(
                "Embedding model '{}' returned no vector for the query",
                self.model_id
            )
        })
    }
}

/// A completion model answering the engine's summary, merge, Now block and fact requests.
pub struct ModelLlm {
    model: Arc<dyn ModelLogic>,
    model_name: String,
}

impl ModelLlm {
    pub fn new(model: Arc<dyn ModelLogic>, model_name: impl Into<String>) -> Self {
        Self {
            model,
            model_name: model_name.into(),
        }
    }

    fn history(&self, request: &LlmRequest, structured: bool) -> History {
        let mut history = History::new(
            self.model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, &request.system),
                HistoryMessage::from_string(Role::User, &request.prompt),
            ],
        );
        history.stream = Some(false);
        history.temperature = Some(0.0);
        history.max_completion_tokens = Some(request.max_output_tokens);
        if structured && let Some(schema) = &request.json_schema {
            history.response_format = Some(ResponseFormat::Object(json!({
                "type": "json_schema",
                "json_schema": { "name": "answer", "schema": schema, "strict": false }
            })));
        }
        history
    }

    async fn ask(&self, history: &History) -> Result<String> {
        let response = self.model.invoke(history, None).await?;
        response
            .last_message()
            .and_then(|message| message.content.clone())
            .filter(|content| !content.trim().is_empty())
            .ok_or_else(|| anyhow!("The memory model '{}' returned no text", self.model_name))
    }
}

#[async_trait]
impl MemoryLlm for ModelLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        if request.json_schema.is_some() {
            match self.ask(&self.history(&request, true)).await {
                Ok(answer) => return Ok(answer),
                Err(error) => tracing::debug!(
                    "The memory model refused a structured answer; asking for plain JSON: {error:#}"
                ),
            }
        }
        self.ask(&self.history(&request, false)).await
    }
}

/// Uses the bot's decision model for fact disposition while preserving its completion model
/// for summaries, extraction and the fallback when a decision cannot be used.
pub struct DecisionLlm {
    completion: Arc<dyn MemoryLlm>,
    decision: Arc<dyn SystemOneModelLogic>,
}

impl DecisionLlm {
    pub fn new(completion: Arc<dyn MemoryLlm>, decision: Arc<dyn SystemOneModelLogic>) -> Self {
        Self {
            completion,
            decision,
        }
    }
}

#[async_trait]
impl MemoryLlm for DecisionLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        self.completion.complete(request).await
    }

    async fn classify_fact(&self, request: &SystemOneRequest) -> Result<Option<SystemOneResponse>> {
        let response = flow_like_types::tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.decision.invoke(request),
        )
        .await
        .map_err(|_| anyhow!("The memory decision model timed out"))??;
        response.validate_for(request)?;
        Ok(Some(response))
    }
}

/// Sends each [`LlmTask`] to its own model, the default for the rest: idle work such as
/// fact extraction may run on a larger model than summaries.
pub struct RoutedLlm {
    default: Arc<dyn MemoryLlm>,
    routes: HashMap<LlmTask, Arc<dyn MemoryLlm>>,
}

impl RoutedLlm {
    pub fn new(default: Arc<dyn MemoryLlm>) -> Self {
        Self {
            default,
            routes: HashMap::new(),
        }
    }

    pub fn route(mut self, task: LlmTask, llm: Arc<dyn MemoryLlm>) -> Self {
        self.routes.insert(task, llm);
        self
    }
}

#[async_trait]
impl MemoryLlm for RoutedLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        let llm = self.routes.get(&request.task).unwrap_or(&self.default);
        llm.complete(request).await
    }

    async fn classify_fact(&self, request: &SystemOneRequest) -> Result<Option<SystemOneResponse>> {
        self.routes
            .get(&LlmTask::FactUpdate)
            .unwrap_or(&self.default)
            .classify_fact(request)
            .await
    }
}

/// The model of an engine that lives longer than one turn's credentials: [`Self::set`] swaps in
/// the next turn's.
pub struct SwitchLlm(RwLock<Arc<dyn MemoryLlm>>);

impl SwitchLlm {
    pub fn new(llm: Arc<dyn MemoryLlm>) -> Self {
        Self(RwLock::new(llm))
    }

    pub fn set(&self, llm: Arc<dyn MemoryLlm>) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = llm;
    }

    fn current(&self) -> Arc<dyn MemoryLlm> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl MemoryLlm for SwitchLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        self.current().complete(request).await
    }

    async fn classify_fact(&self, request: &SystemOneRequest) -> Result<Option<SystemOneResponse>> {
        self.current().classify_fact(request).await
    }
}

/// Like [`SwitchLlm`] for the embedder of one model: a turn's fresh credentials replace the
/// old ones, a different model gets an engine of its own.
pub struct SwitchEmbedder {
    model_id: String,
    current: RwLock<Arc<dyn Embedder>>,
}

impl SwitchEmbedder {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self {
            model_id: embedder.model_id().to_string(),
            current: RwLock::new(embedder),
        }
    }

    /// Swaps in `embedder` when it is the same model; returns whether it was.
    pub fn set(&self, embedder: Arc<dyn Embedder>) -> bool {
        if embedder.model_id() != self.model_id {
            return false;
        }
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = embedder;
        true
    }

    fn current(&self) -> Arc<dyn Embedder> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl Embedder for SwitchEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.current().embed_documents(texts).await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.current().embed_query(text).await
    }
}
