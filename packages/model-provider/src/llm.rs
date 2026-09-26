use anyhow::Result;
use anyhow::anyhow;
use async_trait::async_trait;
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
use serde_json::Value;
use std::collections::HashMap;
use std::{future::Future, pin::Pin, sync::Arc};

use super::{
    history::History,
    response::{Response, Usage as ResponseUsage},
    response_chunk::ResponseChunk,
};

pub mod anthropic;
pub mod bedrock;
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
pub mod mlx;
pub mod moonshot;
pub mod mozilla;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod perplexity;
#[cfg(test)]
pub(crate) mod test_support;
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

/// What an agent needs from a History: body parameters plus the fields Rig's agent builder
/// sets natively.
#[derive(Clone, Debug, Default)]
pub struct AgentSettings {
    pub params: Option<Value>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
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
    fn additional_params(&self, _history: &Option<History>) -> Option<serde_json::Value> {
        None
    }
    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::None
    }

    fn transform_history(&self, _history: &mut History) {}

    /// Body parameters for a request built from `history`: the provider's own mapping, or the
    /// OpenAI-style History fields for providers that take them as they are.
    fn request_params(&self, history: &History, is_streaming: bool) -> Result<Option<Value>> {
        let params = match self.additional_params(&Some(history.clone())) {
            Some(params) => Some(params),
            None => history.build_additional_params()?,
        };
        Ok(apply_usage_reporting(
            params,
            self.usage_reporting(),
            is_streaming,
        ))
    }

    /// Settings for a Rig agent built from `history`, prepared the way `invoke` prepares its
    /// request. The agent picks the transport per call, so the History's `stream` flag must not
    /// turn a completion into SSE.
    fn agent_settings(&self, history: &Option<History>) -> Result<AgentSettings> {
        let Some(history) = history else {
            return Ok(AgentSettings {
                params: apply_usage_reporting(None, self.usage_reporting(), false),
                ..AgentSettings::default()
            });
        };
        let mut history = history.clone();
        self.transform_history(&mut history);
        history.stream = None;
        Ok(AgentSettings {
            params: self.request_params(&history, false)?,
            temperature: history.temperature.map(f64::from),
            max_tokens: history.max_completion_tokens.map(u64::from),
        })
    }

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
        let should_stream = lambda.is_some() && history.stream.unwrap_or(true);
        // The transport method, rather than an additional provider parameter, is authoritative.
        // Keeping this in sync also prevents a non-stream request from carrying `stream: true`.
        history.stream = Some(should_stream);

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

        let model_additional_params = self.request_params(&history, should_stream)?;

        if should_stream {
            invoke_with_stream(
                builder,
                lambda.expect("streaming requires a callback"),
                &model_name,
                model_additional_params,
            )
            .await
        } else {
            let response =
                invoke_without_stream(builder, &model_name, model_additional_params).await?;
            if let Some(callback) = lambda {
                emit_response_to_callback(&response, callback, &model_name).await?;
            }
            Ok(response)
        }
    }
}

pub trait CompletionModelDyn: WasmCompatSend + WasmCompatSync {
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<CompletionResponse<DynamicResponse>, CompletionError>,
    >;

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
    T: CompletionModel<StreamingResponse = R> + Clone + 'static,
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
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<CompletionResponse<DynamicResponse>, CompletionError>,
    > {
        let max_tokens = requested_output_budget(&request);
        Box::pin(async move {
            let resp = CompletionModel::completion(self, request).await?;
            let has_tool_call = resp
                .choice
                .iter()
                .any(|content| matches!(content, rig::message::AssistantContent::ToolCall(_)));
            let finish_reason = resolve_finish_reason(
                reported_finish_reason(&resp.raw_response),
                max_tokens,
                resp.usage.output_tokens,
                has_tool_call,
            );
            Ok(CompletionResponse {
                choice: resp.choice,
                usage: resp.usage,
                raw_response: DynamicResponse {
                    finish_reason: Some(finish_reason),
                },
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
        let max_tokens = requested_output_budget(&request);
        Box::pin(async move {
            let mut has_tool_call = false;
            let stream = CompletionModel::stream(self, request)
                .await?
                .flat_map(move |item| {
                    futures::stream::iter(match item {
                        Ok(item) => {
                            has_tool_call |=
                                matches!(item, StreamedAssistantContent::ToolCall { .. });
                            dynamic_raw_stream_items(item, max_tokens, has_tool_call)
                                .into_iter()
                                .map(Ok)
                                .collect::<Vec<_>>()
                        }
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
    M: CompletionModel<StreamingResponse = R> + Clone + 'static,
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
    type Response = DynamicResponse;
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
    /// OpenAI vocabulary (`stop`, `length`, `tool_calls`, `content_filter`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

impl GetTokenUsage for DynamicStreamingResponse {
    fn token_usage(&self) -> Option<RigUsage> {
        self.usage
    }
}

/// Maps a provider's stop reason onto the OpenAI vocabulary the `Response` protocol uses.
fn normalize_finish_reason(reason: &str) -> Option<String> {
    let reason = reason.trim().to_ascii_lowercase();
    let normalized = match reason.as_str() {
        "" | "null" | "finish_reason_unspecified" => return None,
        "length"
        | "max_tokens"
        | "max_output_tokens"
        | "model_length"
        | "model_context_window_exceeded" => "length",
        "tool_calls" | "tool_call" | "tool_use" | "function_call" => "tool_calls",
        "content_filter" | "safety" | "recitation" | "blocklist" | "prohibited_content"
        | "spii" | "refusal" => "content_filter",
        "stop" | "end_turn" | "stop_sequence" | "complete" => "stop",
        _ => return Some(reason),
    };
    Some(normalized.to_string())
}

/// Reads the stop reason from a serialized Rig raw response. Rig 0.38.2 keeps it only in
/// provider-specific shapes: OpenAI-compatible `choices`, Anthropic `stop_reason`, Gemini
/// `candidates`/`finish_reason`, Ollama `done_reason`, Cohere and our own dynamic responses.
fn reported_finish_reason(raw: &impl Serialize) -> Option<String> {
    let raw = serde_json::to_value(raw).ok()?;
    if raw.get("status").and_then(Value::as_str) == Some("incomplete") {
        let reason = raw
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("length");
        return normalize_finish_reason(reason);
    }
    [
        "/choices/0/finish_reason",
        "/candidates/0/finishReason",
        "/finish_reason",
        "/stop_reason",
        "/done_reason",
    ]
    .iter()
    .find_map(|pointer| raw.pointer(pointer).and_then(Value::as_str))
    .and_then(normalize_finish_reason)
}

/// An output that consumed the whole requested budget was truncated, whatever the provider
/// (or a Rig client that discards the reason) says.
fn resolve_finish_reason(
    reported: Option<String>,
    max_tokens: Option<u64>,
    output_tokens: u64,
    has_tool_call: bool,
) -> String {
    let budget_exhausted = max_tokens.is_some_and(|budget| budget > 0 && output_tokens >= budget);
    match reported {
        Some(reason) if !(budget_exhausted && reason == "stop") => reason,
        _ if budget_exhausted => "length".to_string(),
        _ if has_tool_call => "tool_calls".to_string(),
        _ => "stop".to_string(),
    }
}

fn dynamic_raw_stream_items<R>(
    item: StreamedAssistantContent<R>,
    max_tokens: Option<u64>,
    has_tool_call: bool,
) -> Vec<RawStreamingChoice<DynamicStreamingResponse>>
where
    R: Clone + Unpin + GetTokenUsage + Serialize,
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
            let usage = response.token_usage();
            let finish_reason = resolve_finish_reason(
                reported_finish_reason(&response),
                max_tokens,
                usage.map_or(0, |usage| usage.output_tokens),
                has_tool_call,
            );
            vec![RawStreamingChoice::FinalResponse(
                DynamicStreamingResponse {
                    usage,
                    finish_reason: Some(finish_reason),
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

    /// For Rig clients whose request body omits `CompletionRequest::max_tokens` (Rig 0.38.2:
    /// OpenRouter, Azure and most OpenAI-compatible providers). Without it, the provider or the
    /// hosted relay applies its own default budget.
    pub fn with_max_tokens_body_param(
        client: impl CompletionClientDyn + Send + Sync + 'static,
    ) -> Self {
        Self::with_request_fixup(client, |_, request| {
            output_budget_as_body_param(request, "max_tokens")
        })
    }

    /// Enforces a provider's constraints on the final Rig request. It runs for `invoke` and for
    /// agents alike, and sees fields (`tool_choice`, `temperature`) that agents set on the
    /// builder rather than on the History.
    pub fn with_request_fixup(
        client: impl CompletionClientDyn + Send + Sync + 'static,
        fixup: RequestFixup,
    ) -> Self {
        Self {
            inner: Box::new(FixupClient {
                client: Box::new(client),
                fixup,
            }),
        }
    }
}

/// Rewrites a request for the named model right before it reaches the provider client.
pub type RequestFixup = fn(&str, CompletionRequest) -> CompletionRequest;

/// How a provider names, or lacks, the OpenAI-style fields `History::build_additional_params`
/// emits. Strict providers reject unknown fields, which fails the whole request.
pub struct ParamDialect {
    pub provider: &'static str,
    pub renames: &'static [(&'static str, &'static str)],
    pub unsupported: &'static [&'static str],
    /// Fields the Rig client sets itself from the transport.
    pub rig_owned: &'static [&'static str],
}

/// OpenRouter-only History fields.
pub const OPENROUTER_ONLY: [&str; 2] = ["usage", "preset"];

impl ParamDialect {
    pub fn params(&self, history: &History) -> Value {
        let mut params = history
            .build_additional_params()
            .ok()
            .flatten()
            .unwrap_or_else(|| Value::Object(Default::default()));
        if let Some(object) = params.as_object_mut() {
            for key in self.rig_owned {
                object.remove(*key);
            }
            for key in self.unsupported {
                if object.remove(*key).is_some() {
                    tracing::warn!(
                        provider = self.provider,
                        parameter = key,
                        "Provider has no such request parameter; dropping it"
                    );
                }
            }
            for (from, to) in self.renames {
                if let Some(value) = object.remove(*from) {
                    object.insert((*to).to_string(), value);
                }
            }
        }
        params
    }
}

/// The request's body parameters as an object, created when absent.
pub fn body_params(request: &mut CompletionRequest) -> &mut serde_json::Map<String, Value> {
    let params = request
        .additional_params
        .get_or_insert_with(|| Value::Object(Default::default()));
    if !params.is_object() {
        *params = Value::Object(Default::default());
    }
    params.as_object_mut().expect("ensured object")
}

/// Removes a body parameter the model rejects, saying why.
pub fn drop_body_param(request: &mut CompletionRequest, key: &str, model: &str, reason: &str) {
    if let Some(params) = request
        .additional_params
        .as_mut()
        .and_then(Value::as_object_mut)
        && params.remove(key).is_some()
    {
        tracing::warn!(model, parameter = key, reason, "Dropping request parameter");
    }
}

/// For models that reject forced `tool_choice`: lets the model choose and names the required
/// tool in the system prompt instead, the providers' documented workaround.
pub fn unforce_tool_choice(request: &mut CompletionRequest, model: &str) {
    let instruction = match request.tool_choice.as_ref() {
        Some(rig::message::ToolChoice::Required) => {
            "Respond by calling one of the provided tools.".to_string()
        }
        Some(rig::message::ToolChoice::Specific { function_names }) => format!(
            "Respond by calling the {} tool.",
            function_names
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(" or ")
        ),
        _ => return,
    };
    request.tool_choice = Some(rig::message::ToolChoice::Auto);
    request.preamble = Some(match request.preamble.take() {
        Some(preamble) if !preamble.is_empty() => format!("{preamble}\n\n{instruction}"),
        _ => instruction,
    });
    tracing::debug!(
        model,
        "Model rejects forced tool_choice; sending auto with an instruction naming the tool"
    );
}

/// Removes `temperature` when the model rejects it, saying why.
pub fn drop_temperature(request: &mut CompletionRequest, model: &str, reason: &str) {
    if request.temperature.take().is_some() {
        tracing::warn!(
            model,
            parameter = "temperature",
            reason,
            "Dropping request parameter"
        );
    }
}

fn requested_output_budget(request: &CompletionRequest) -> Option<u64> {
    request.max_tokens.or_else(|| {
        let params = request.additional_params.as_ref()?;
        ["max_completion_tokens", "max_tokens", "num_predict"]
            .iter()
            .find_map(|key| params.get(*key).and_then(Value::as_u64))
    })
}

/// Copies `CompletionRequest::max_tokens` into the body under the provider's own name.
pub fn output_budget_as_body_param(mut request: CompletionRequest, key: &str) -> CompletionRequest {
    if let Some(max_tokens) = request.max_tokens {
        request.additional_params = merge_additional_params(
            request.additional_params.take(),
            Some(serde_json::json!({ key: max_tokens })),
        );
    }
    request
}

struct FixupClient {
    client: Box<dyn CompletionClientDyn + Send + Sync>,
    fixup: RequestFixup,
}

impl FixupClient {
    fn model<'a>(&self, model: &str) -> FixupModel<'a> {
        FixupModel {
            inner: Arc::from(self.client.completion_model(model)),
            model: model.to_string(),
            fixup: self.fixup,
        }
    }
}

impl CompletionClientDyn for FixupClient {
    fn completion_model<'a>(&self, model: &str) -> Box<dyn CompletionModelDyn + 'a> {
        Box::new(self.model(model))
    }

    fn agent<'a>(&self, model: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
        AgentBuilder::new(CompletionModelHandle::new(Arc::new(self.model(model))))
    }
}

#[derive(Clone)]
struct FixupModel<'a> {
    inner: Arc<dyn CompletionModelDyn + 'a>,
    model: String,
    fixup: RequestFixup,
}

impl FixupModel<'_> {
    fn fix(&self, request: CompletionRequest) -> CompletionRequest {
        let model = request.model.clone().unwrap_or_else(|| self.model.clone());
        (self.fixup)(&model, request)
    }
}

impl CompletionModelDyn for FixupModel<'_> {
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<CompletionResponse<DynamicResponse>, CompletionError>,
    > {
        self.inner.completion(self.fix(request))
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        std::result::Result<StreamingCompletionResponse<DynamicStreamingResponse>, CompletionError>,
    > {
        self.inner.stream(self.fix(request))
    }

    fn completion_request(
        &self,
        prompt: Message,
    ) -> CompletionRequestBuilder<CompletionModelHandle<'_>> {
        CompletionRequestBuilder::new(CompletionModelHandle::new(Arc::new(self.clone())), prompt)
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

/// Provider-independent remainder of a Rig raw response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynamicResponse {
    /// OpenAI vocabulary (`stop`, `length`, `tool_calls`, `content_filter`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

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
        async move { model.completion(request).await }
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
    additional_params: Option<serde_json::Value>,
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
    if let Some(finish_reason) = completion.raw_response.finish_reason
        && let Some(choice) = response.choices.first_mut()
    {
        choice.finish_reason = finish_reason;
    }
    Ok(response)
}

/// Replays a structured non-stream response through the callback interface. Rig's stream API has
/// no media event, so this is also how callers that explicitly disable streaming can still observe
/// a generated image before the finish chunk.
pub(crate) async fn emit_response_to_callback(
    response: &Response,
    callback: LLMCallback,
    model_name: &str,
) -> Result<()> {
    if let Message::Assistant { content, .. } = response.to_rig_message()? {
        for item in content.iter() {
            let chunk = match item {
                rig::message::AssistantContent::Text(text) => {
                    ResponseChunk::from_text(&text.text, model_name)
                }
                rig::message::AssistantContent::ToolCall(tool_call) => {
                    ResponseChunk::from_tool_call(tool_call, model_name)
                }
                rig::message::AssistantContent::Reasoning(reasoning) => {
                    ResponseChunk::from_reasoning(&reasoning.display_text(), model_name)
                }
                rig::message::AssistantContent::Image(image) => ResponseChunk::from_content_part(
                    super::history::Content::from_rig_image(image.clone()),
                    model_name,
                ),
            };
            callback(chunk).await?;
        }
    }

    let finish_reason = response
        .choices
        .first()
        .map(|choice| choice.finish_reason.as_str())
        .filter(|reason| !reason.is_empty());
    let mut finish = ResponseChunk::finish(model_name, None, finish_reason);
    finish.usage = Some(response.usage.clone());
    callback(finish).await
}

#[allow(deprecated)]
async fn invoke_with_stream<'a>(
    builder: CompletionRequestBuilder<CompletionModelHandle<'a>>,
    callback: LLMCallback,
    model_name: &str,
    additional_params: Option<serde_json::Value>,
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
    let mut finish_reason: Option<String> = None;
    let mut streamed_reasoning = String::new();

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
                let chunk = match content {
                    ToolCallDeltaContent::Name(name) => {
                        ResponseChunk::from_tool_call_name_delta(&id, &name, model_name)
                    }
                    ToolCallDeltaContent::Delta(delta) => {
                        ResponseChunk::from_tool_call_delta(&id, &delta, model_name)
                    }
                };
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                // Some providers stream ReasoningDelta and then replay the
                // whole accumulated block as a final Reasoning item, others
                // send one Reasoning per chunk — emit only what is new, with
                // no separator (chunks are token fragments).
                let reasoning_text = reasoning
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        rig::message::ReasoningContent::Text { text, .. } => Some(text.as_str()),
                        rig::message::ReasoningContent::Summary(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let new_text = if let Some(suffix) =
                    reasoning_text.strip_prefix(streamed_reasoning.as_str())
                {
                    suffix
                } else if streamed_reasoning.ends_with(reasoning_text.as_str()) {
                    ""
                } else {
                    reasoning_text.as_str()
                };
                if !new_text.is_empty() {
                    let chunk = ResponseChunk::from_reasoning(new_text, model_name);
                    response.push_chunk(chunk.clone());
                    streamed_reasoning.push_str(new_text);
                    callback(chunk).await?;
                }
            }
            StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                streamed_reasoning.push_str(&reasoning);
                let chunk = ResponseChunk::from_reasoning(&reasoning, model_name);
                response.push_chunk(chunk.clone());
                callback(chunk).await?;
            }
            StreamedAssistantContent::Final(final_resp) => {
                final_usage = final_resp.usage;
                finish_reason = final_resp.finish_reason;
            }
        }
    }

    let finish_chunk =
        ResponseChunk::finish(model_name, final_usage.as_ref(), finish_reason.as_deref());
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
    use serde_json::json;

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
    fn reported_finish_reasons_are_read_from_each_provider_shape() {
        for (raw, expected) in [
            (json!({"choices": [{"finish_reason": "length"}]}), "length"),
            (json!({"stop_reason": "max_tokens"}), "length"),
            (
                json!({"candidates": [{"finishReason": "MAX_TOKENS"}]}),
                "length",
            ),
            (json!({"finish_reason": "MAX_TOKENS"}), "length"),
            (json!({"done_reason": "length"}), "length"),
            (
                json!({"status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}}),
                "length",
            ),
            (json!({"stop_reason": "tool_use"}), "tool_calls"),
            (json!({"stop_reason": "end_turn"}), "stop"),
            (
                json!({"candidates": [{"finishReason": "SAFETY"}]}),
                "content_filter",
            ),
            (json!({"stop_reason": "pause_turn"}), "pause_turn"),
        ] {
            assert_eq!(
                reported_finish_reason(&raw).as_deref(),
                Some(expected),
                "{raw}"
            );
        }
        assert_eq!(
            reported_finish_reason(&json!({"status": "completed"})),
            None
        );
        assert_eq!(reported_finish_reason(&()), None);
    }

    #[test]
    fn exhausted_budget_is_length_even_when_reported_as_stop() {
        let reported = || Some("stop".to_string());
        assert_eq!(
            resolve_finish_reason(reported(), Some(100_000), 100_000, false),
            "length"
        );
        assert_eq!(
            resolve_finish_reason(None, Some(4096), 4096, true),
            "length"
        );
        assert_eq!(
            resolve_finish_reason(reported(), Some(100_000), 4096, false),
            "stop"
        );
        assert_eq!(
            resolve_finish_reason(Some("length".into()), Some(100_000), 4096, false),
            "length"
        );
        assert_eq!(resolve_finish_reason(None, None, 4096, true), "tool_calls");
        assert_eq!(resolve_finish_reason(None, None, 0, false), "stop");
    }

    #[test]
    fn max_tokens_body_param_keeps_existing_params() {
        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: Some(100_000),
            tool_choice: None,
            additional_params: Some(json!({"stream": true, "usage": {"include": true}})),
            output_schema: None,
        };

        let params = output_budget_as_body_param(request, "max_tokens")
            .additional_params
            .unwrap();

        assert_eq!(
            params,
            json!({"stream": true, "usage": {"include": true}, "max_tokens": 100_000})
        );
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
