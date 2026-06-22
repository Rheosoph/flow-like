use flow_like_types::async_trait;
use flow_like_types::{Result, Value, anyhow};
use futures::StreamExt;
use http::{HeaderMap, HeaderName, HeaderValue};
use rig::agent::AgentBuilder;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionRequestBuilder,
    CompletionResponse, GetTokenUsage, Message, Usage as RigUsage,
};
use rig::streaming::{
    RawStreamingChoice, RawStreamingToolCall, StreamedAssistantContent,
    StreamingCompletionResponse, ToolCallDeltaContent,
};
use rig::wasm_compat::{WasmBoxedFuture, WasmCompatSend, WasmCompatSync};
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
pub mod vertex;
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

pub fn merge_additional_params(base: Option<Value>, extra: Option<Value>) -> Option<Value> {
    match (base, extra) {
        (None, None) => None,
        (Some(base), None) => Some(base),
        (None, Some(extra)) => Some(extra),
        (Some(mut base), Some(extra)) => {
            if let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) {
                for (key, value) in extra_obj {
                    base_obj.insert(key.clone(), value.clone());
                }
                Some(base)
            } else {
                Some(extra)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageReportingMode {
    #[default]
    None,
    OpenAIStreamOptions,
    OpenRouterUsageInclude,
}

fn ensure_object_param(mut params: Option<Value>) -> Value {
    match params.take() {
        Some(value) if value.is_object() => value,
        _ => Value::Object(Default::default()),
    }
}

fn enable_openai_stream_usage(mut params: Option<Value>, is_streaming: bool) -> Option<Value> {
    if !is_streaming {
        return params;
    }

    let mut value = ensure_object_param(params.take());
    let Some(obj) = value.as_object_mut() else {
        return Some(value);
    };

    let stream_options = obj
        .entry("stream_options".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    if !stream_options.is_object() {
        *stream_options = Value::Object(Default::default());
    }
    if let Some(stream_options) = stream_options.as_object_mut() {
        stream_options.insert("include_usage".to_string(), Value::Bool(true));
    }

    Some(value)
}

fn enable_openrouter_usage_include(mut params: Option<Value>) -> Option<Value> {
    let mut value = ensure_object_param(params.take());
    let Some(obj) = value.as_object_mut() else {
        return Some(value);
    };

    let usage = obj
        .entry("usage".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    if !usage.is_object() {
        *usage = Value::Object(Default::default());
    }
    if let Some(usage) = usage.as_object_mut() {
        usage.insert("include".to_string(), Value::Bool(true));
    }

    Some(value)
}

fn apply_usage_reporting(
    params: Option<Value>,
    mode: UsageReportingMode,
    is_streaming: bool,
) -> Option<Value> {
    match mode {
        UsageReportingMode::None => params,
        UsageReportingMode::OpenAIStreamOptions => enable_openai_stream_usage(params, is_streaming),
        UsageReportingMode::OpenRouterUsageInclude => enable_openrouter_usage_include(params),
    }
}

#[async_trait]
pub trait ModelLogic: Send + Sync {
    async fn provider(&self) -> Result<ModelConstructor>;
    async fn default_model(&self) -> Option<String>;
    fn additional_params(&self, _history: &Option<History>) -> Option<flow_like_types::Value> {
        None
    }
    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::None
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
        let mut model_additional_params = self.additional_params(&Some(history.clone()));
        if model_additional_params.is_none() {
            model_additional_params = history.build_additional_params()?;
        }
        model_additional_params = apply_usage_reporting(
            model_additional_params,
            self.usage_reporting(),
            lambda.is_some(),
        );

        if let Some(callback) = lambda {
            invoke_with_stream(builder, callback, &model_name, model_additional_params).await
        } else {
            invoke_without_stream(builder, &model_name, model_additional_params).await
        }
    }
}

pub trait CompletionModelDyn: WasmCompatSend + WasmCompatSync {
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<'_, std::result::Result<CompletionResponse<()>, CompletionError>>;

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<StreamingCompletionResponse<DynamicStreamingResponse>, CompletionError>,
    >;

    fn completion_request(
        &self,
        prompt: Message,
    ) -> CompletionRequestBuilder<CompletionModelHandle<'_>>;
}

impl<T, R> CompletionModelDyn for T
where
    T: CompletionModel<StreamingResponse = R> + 'static,
    R: Clone
        + Unpin
        + GetTokenUsage
        + Send
        + Sync
        + Serialize
        + for<'de> Deserialize<'de>
        + 'static,
{
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<'_, std::result::Result<CompletionResponse<()>, CompletionError>> {
        Box::pin(async move {
            self.completion(request)
                .await
                .map(|resp| CompletionResponse {
                    choice: resp.choice,
                    usage: resp.usage,
                    raw_response: (),
                    message_id: resp.message_id,
                })
        })
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<StreamingCompletionResponse<DynamicStreamingResponse>, CompletionError>,
    > {
        Box::pin(async move {
            let stream = self.stream(request).await?.flat_map(|item| {
                futures::stream::iter(match item {
                    Ok(item) => dynamic_raw_stream_items(item)
                        .into_iter()
                        .map(Ok)
                        .collect::<Vec<_>>(),
                    Err(err) => vec![Err(err)],
                })
            });

            Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
        })
    }

    fn completion_request(
        &self,
        prompt: Message,
    ) -> CompletionRequestBuilder<CompletionModelHandle<'_>> {
        CompletionRequestBuilder::new(CompletionModelHandle::new(Arc::new(self.clone())), prompt)
    }
}

pub trait CompletionClientDyn {
    fn completion_model<'a>(&self, model: &str) -> Box<dyn CompletionModelDyn + 'a>;
    fn agent<'a>(&self, model: &str) -> AgentBuilder<CompletionModelHandle<'a>>;
}

impl<T, M, R> CompletionClientDyn for T
where
    T: rig::client::CompletionClient<CompletionModel = M>,
    M: CompletionModel<StreamingResponse = R> + 'static,
    R: Clone
        + Unpin
        + GetTokenUsage
        + Send
        + Sync
        + Serialize
        + for<'de> Deserialize<'de>
        + 'static,
{
    fn completion_model<'a>(&self, model: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(rig::client::CompletionClient::completion_model(self, model))
    }

    fn agent<'a>(&self, model: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        AgentBuilder::new(CompletionModelHandle::new(Arc::new(
            rig::client::CompletionClient::completion_model(self, model),
        )))
    }
}

#[derive(Clone)]
pub struct CompletionModelHandle<'a>(Arc<dyn CompletionModelDyn + 'a>);

impl<'a> CompletionModelHandle<'a> {
    pub fn new(handle: Arc<dyn CompletionModelDyn + 'a>) -> Self {
        Self(handle)
    }
}

impl CompletionModel for CompletionModelHandle<'_> {
    type Response = ();
    type StreamingResponse = DynamicStreamingResponse;
    type Client = ();

    fn make(_: &Self::Client, _: impl Into<String>) -> Self {
        panic!("Cannot create a completion model handle from a client")
    }

    fn completion(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<CompletionResponse<Self::Response>, CompletionError>,
    > + WasmCompatSend {
        self.0.completion(request)
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<
            StreamingCompletionResponse<Self::StreamingResponse>,
            CompletionError,
        >,
    > + WasmCompatSend {
        self.0.stream(request)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynamicStreamingResponse {
    pub usage: Option<RigUsage>,
}

impl GetTokenUsage for DynamicStreamingResponse {
    fn token_usage(&self) -> Option<RigUsage> {
        self.usage.clone()
    }
}

fn dynamic_raw_stream_items<R>(
    item: StreamedAssistantContent<R>,
) -> Vec<RawStreamingChoice<DynamicStreamingResponse>>
where
    R: Clone + Unpin + GetTokenUsage,
{
    match item {
        StreamedAssistantContent::Text(text) => {
            let mut items = Vec::new();
            if let Some(additional_params) = text.additional_params {
                items.push(RawStreamingChoice::TextStart {
                    additional_params: Some(additional_params),
                });
            }
            items.push(RawStreamingChoice::Message(text.text));
            items
        }
        StreamedAssistantContent::ToolCall {
            tool_call,
            internal_call_id,
        } => vec![RawStreamingChoice::ToolCall(RawStreamingToolCall {
            id: tool_call.id,
            internal_call_id,
            call_id: tool_call.call_id,
            name: tool_call.function.name,
            arguments: tool_call.function.arguments,
            signature: tool_call.signature,
            additional_params: tool_call.additional_params,
        })],
        StreamedAssistantContent::ToolCallDelta {
            id,
            internal_call_id,
            content,
        } => vec![RawStreamingChoice::ToolCallDelta {
            id,
            internal_call_id,
            content,
        }],
        StreamedAssistantContent::Reasoning(reasoning) => reasoning
            .content
            .into_iter()
            .map(|content| RawStreamingChoice::Reasoning {
                id: reasoning.id.clone(),
                content,
            })
            .collect(),
        StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
            vec![RawStreamingChoice::ReasoningDelta { id, reasoning }]
        }
        StreamedAssistantContent::Final(response) => {
            vec![RawStreamingChoice::FinalResponse(
                DynamicStreamingResponse {
                    usage: response.token_usage(),
                },
            )]
        }
    }
}

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
    type StreamingResponse = DynamicStreamingResponse;
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
                message_id: response.message_id,
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
            StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id: _,
            } => {
                let chunk = ResponseChunk::from_tool_call(&tool_call, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::ToolCallDelta {
                id,
                content,
                internal_call_id: _,
            } => {
                let delta_str = match content {
                    ToolCallDeltaContent::Name(name) => name,
                    ToolCallDeltaContent::Delta(delta) => delta,
                };
                let chunk = ResponseChunk::from_tool_call_delta(&id, &delta_str, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                let reasoning_text = reasoning
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        rig::message::ReasoningContent::Text { text, .. } => Some(text.as_str()),
                        rig::message::ReasoningContent::Summary(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn openai_stream_usage_preserves_existing_stream_options() {
        let params = Some(json!({
            "stream_options": {
                "include_obfuscation": false
            }
        }));

        let result =
            apply_usage_reporting(params, UsageReportingMode::OpenAIStreamOptions, true).unwrap();

        assert_eq!(
            result
                .get("stream_options")
                .and_then(|v| v.get("include_usage"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            result
                .get("stream_options")
                .and_then(|v| v.get("include_obfuscation"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn openai_stream_usage_skips_non_streaming_requests() {
        let params = apply_usage_reporting(None, UsageReportingMode::OpenAIStreamOptions, false);
        assert!(params.is_none());
    }

    #[test]
    fn openrouter_usage_include_is_always_enabled() {
        let result =
            apply_usage_reporting(None, UsageReportingMode::OpenRouterUsageInclude, false).unwrap();
        assert_eq!(
            result
                .get("usage")
                .and_then(|v| v.get("include"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }
}
