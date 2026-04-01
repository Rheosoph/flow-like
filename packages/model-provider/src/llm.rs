use flow_like_types::async_trait;
use flow_like_types::{Result, Value, anyhow};
use futures::StreamExt;
use http::{HeaderMap, HeaderName, HeaderValue};
use rig::client::FinalCompletionResponse;
#[allow(deprecated)]
pub use rig::client::completion::{CompletionClientDyn, CompletionModelHandle};
#[allow(deprecated)]
use rig::completion::CompletionModelDyn;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionRequestBuilder,
    CompletionResponse, Message, Usage as RigUsage,
};
use rig::streaming::{StreamedAssistantContent, StreamingCompletionResponse, ToolCallDeltaContent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{future::Future, pin::Pin, sync::Arc};

use super::{
    history::History,
    response::{Response, Usage as ResponseUsage},
    response_chunk::ResponseChunk,
};

// pub mod bedrock;
pub mod anthropic;
pub mod cohere;
pub mod deepseek;
pub mod galadriel;
pub mod gemini;
pub mod groq;
pub mod huggingface;
pub mod hyperbolic;
pub mod llamacpp;
pub mod lmstudio;
pub mod mira;
pub mod mistral;
pub mod moonshot;
pub mod mozilla;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod perplexity;
pub mod together;
pub mod voyageai;
pub mod xai;

pub type LLMCallback = Arc<
    dyn Fn(ResponseChunk) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>
        + Send
        + Sync
        + 'static,
>;

/// Extract custom HTTP headers from provider params
/// Expects a "headers" key containing an object with header name-value pairs
pub fn extract_headers(params: &HashMap<String, Value>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(headers_obj) = params.get("headers").and_then(|v| v.as_object()) {
        for (key, value) in headers_obj {
            if let Some(value_str) = value.as_str()
                && let (Ok(name), Ok(val)) = (
                    HeaderName::try_from(key.as_str()),
                    HeaderValue::from_str(value_str),
                )
            {
                headers.insert(name, val);
            }
        }
    }
    headers
}

#[async_trait]
pub trait ModelLogic: Send + Sync {
    async fn provider(&self) -> Result<ModelConstructor>;
    async fn default_model(&self) -> Option<String>;
    fn additional_params(&self, _history: &Option<History>) -> Option<flow_like_types::Value> {
        None
    }

    fn transform_history(&self, _history: &mut History) {}

    /// Get a DynamicCompletionModel for use with external libraries that need `CompletionModel`.
    /// This is the preferred method over `completion_model_handle` as it properly implements the trait.
    #[allow(deprecated)]
    async fn dynamic_completion_model(
        &self,
        model_name: Option<&str>,
    ) -> Result<DynamicCompletionModel> {
        let default = self.default_model().await;
        let model_name = model_name
            .map(|s| s.to_string())
            .or(default)
            .ok_or_else(|| anyhow!("No model name provided and no default model available"))?;

        let constructor = self.provider().await?;
        Ok(constructor.dynamic_model(&model_name))
    }

    /// Get the underlying rig CompletionModelHandle for use with external libraries
    #[deprecated(note = "Use `dynamic_completion_model` instead")]
    #[allow(deprecated)]
    async fn completion_model_handle(
        &self,
        model_name: Option<&str>,
    ) -> Result<CompletionModelHandle<'static>> {
        let default = self.default_model().await;
        let model_name = model_name
            .map(|s| s.to_string())
            .or(default)
            .ok_or_else(|| anyhow!("No model name provided and no default model available"))?;

        let constructor = self.provider().await?;
        let completion_model = constructor.inner.completion_model(&model_name);
        Ok(CompletionModelHandle::new(Arc::from(completion_model)))
    }

    #[allow(deprecated)]
    async fn invoke(&self, history: &History, lambda: Option<LLMCallback>) -> Result<Response> {
        let mut history = history.clone();
        self.transform_history(&mut history);
        history.normalize_for_alternation();

        let model_name = self
            .default_model()
            .await
            .unwrap_or_else(|| history.model.clone());

        let constructor = self.provider().await?;
        let completion_model = constructor.inner.completion_model(&model_name);
        let completion_handle = CompletionModelHandle::new(Arc::from(completion_model));

        // Extract and remove system prompt so it becomes preamble instead of
        // being converted to a User message (which would break role alternation
        // for models that require strict user/assistant/user/assistant ordering).
        let system_prompt = history.take_system_prompt();

        let (prompt, chat_history) = history
            .extract_prompt_and_history()
            .map_err(|e| anyhow!("Failed to convert history into rig messages: {e}"))?;

        let mut builder =
            CompletionModel::completion_request(&completion_handle, prompt).messages(chat_history);

        if let Some(preamble) = system_prompt {
            builder = builder.preamble(preamble);
        }

        if let Some(temp) = history.temperature {
            builder = builder.temperature(temp as f64);
        }

        if let Some(max_tokens) = history.max_completion_tokens {
            builder = builder.max_tokens(max_tokens as u64);
        }

        if history.tools.is_some() {
            let tool_definitions = history.tools_to_rig()?;
            if !tool_definitions.is_empty() {
                builder = builder.tools(tool_definitions);
            }
        }

        if let Some(choice) = history.tool_choice_to_rig() {
            builder = builder.tool_choice(choice);
        }

        // Note: We call self.additional_params() later which may need to merge with history params
        // Some providers (like Gemini) need to filter certain fields from history params
        // So we let the model implementation handle the merging in additional_params()
        // Only add history params here if the model doesn't provide custom params
        let model_additional_params = self.additional_params(&Some(history.clone()));

        if model_additional_params.is_none() {
            // Model doesn't provide custom params, use history params directly
            if let Some(params) = history.build_additional_params()? {
                builder = builder.additional_params(params);
            }
        }

        if let Some(callback) = lambda {
            invoke_with_stream(builder, callback, &model_name, model_additional_params).await
        } else {
            invoke_without_stream(builder, &model_name, model_additional_params).await
        }
    }
}

#[allow(deprecated)]
pub struct ModelConstructor {
    pub inner: Box<dyn CompletionClientDyn + Send + Sync>,
}

#[allow(deprecated)]
impl ModelConstructor {
    pub fn client(&self) -> &(dyn CompletionClientDyn + Send + Sync) {
        self.inner.as_ref()
    }

    /// Consumes the constructor and returns the inner completion client
    pub fn into_client(self) -> Box<dyn CompletionClientDyn + Send + Sync> {
        self.inner
    }

    /// Create a DynamicCompletionModel for the given model name.
    /// This properly returns a type that implements `CompletionModel + Send + Sync + 'static`.
    pub fn dynamic_model(self, model_name: &str) -> DynamicCompletionModel {
        DynamicCompletionModel::new(self.inner, model_name.to_string())
    }
}

/// A wrapper around a `CompletionClientDyn` + model name that properly implements `CompletionModel`.
/// This allows using dynamic completion models with libraries that require the concrete trait.
/// The model is created lazily on each request to avoid lifetime issues.
#[derive(Clone)]
#[allow(deprecated)]
pub struct DynamicCompletionModel {
    client: Arc<dyn CompletionClientDyn + Send + Sync>,
    model_name: String,
}

#[allow(deprecated)]
impl DynamicCompletionModel {
    pub fn new(client: Box<dyn CompletionClientDyn + Send + Sync>, model_name: String) -> Self {
        Self {
            client: Arc::from(client),
            model_name,
        }
    }

    pub fn from_arc(
        client: Arc<dyn CompletionClientDyn + Send + Sync>,
        model_name: String,
    ) -> Self {
        Self { client, model_name }
    }
}

/// Response type for dynamic completion models - always returns unit type
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynamicResponse;

#[allow(deprecated)]
impl CompletionModel for DynamicCompletionModel {
    type Response = DynamicResponse;
    type StreamingResponse = FinalCompletionResponse;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        panic!(
            "DynamicCompletionModel cannot be created from a client - use DynamicCompletionModel::new() instead"
        )
    }

    fn completion(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<CompletionResponse<Self::Response>, CompletionError>,
    > + Send {
        let model = self.client.completion_model(&self.model_name);
        async move {
            let response = model.completion(request).await?;
            Ok(CompletionResponse {
                choice: response.choice,
                usage: response.usage,
                raw_response: DynamicResponse,
            })
        }
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>,
    > + Send {
        let model = self.client.completion_model(&self.model_name);
        async move { model.stream(request).await }
    }
}

#[allow(deprecated)]
async fn invoke_without_stream<'a>(
    builder: CompletionRequestBuilder<CompletionModelHandle<'a>>,
    model_name: &str,
    additional_params: Option<flow_like_types::Value>,
) -> Result<Response> {
    let builder = if let Some(params) = additional_params {
        builder.additional_params(params)
    } else {
        builder
    };

    let completion = builder
        .send()
        .await
        .map_err(|e| anyhow!("Rig completion error: {e}"))?;

    let message = Message::Assistant {
        id: None,
        content: completion.choice.clone(),
    };

    let mut response = Response::from_rig_message(message)?;
    response.model = Some(model_name.to_string());
    response.usage = ResponseUsage::from_rig(completion.usage);
    Ok(response)
}

#[allow(deprecated)]
async fn invoke_with_stream<'a>(
    builder: CompletionRequestBuilder<CompletionModelHandle<'a>>,
    callback: LLMCallback,
    model_name: &str,
    additional_params: Option<flow_like_types::Value>,
) -> Result<Response> {
    let builder = if let Some(params) = additional_params {
        builder.additional_params(params)
    } else {
        builder
    };

    let mut stream = builder.stream().await.map_err(|e| {
        // Extract more detailed error information
        let error_msg = format!("{:?}", e);
        anyhow!("Rig streaming error: {} | Details: {}", e, error_msg)
    })?;

    let mut response = Response::new();
    response.model = Some(model_name.to_string());

    let mut final_usage: Option<RigUsage> = None;

    while let Some(item) = stream.next().await {
        let content = item.map_err(|e| {
            let error_msg = format!("{:?}", e);
            anyhow!("Rig streaming error: {} | Details: {}", e, error_msg)
        })?;
        match content {
            StreamedAssistantContent::Text(text) => {
                let chunk = ResponseChunk::from_text(&text.text, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::ToolCall(tool_call) => {
                let chunk = ResponseChunk::from_tool_call(&tool_call, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::ToolCallDelta { id, content } => {
                let delta_str = match content {
                    ToolCallDeltaContent::Name(name) => name,
                    ToolCallDeltaContent::Delta(delta) => delta,
                };
                let chunk = ResponseChunk::from_tool_call_delta(&id, &delta_str, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                // Join the reasoning vec into a single string
                let reasoning_text = reasoning.reasoning.join("\n");
                let chunk = ResponseChunk::from_reasoning(&reasoning_text, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                let chunk = ResponseChunk::from_reasoning(&reasoning, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::Final(final_resp) => {
                final_usage = final_resp.usage;
            }
        }
    }

    let finish_chunk = ResponseChunk::finish(model_name, final_usage.as_ref());
    response.push_chunk(finish_chunk.clone());
    callback(finish_chunk).await?;

    if let Some(usage) = final_usage {
        response.usage = ResponseUsage::from_rig(usage);
    }

    Ok(response)
}
