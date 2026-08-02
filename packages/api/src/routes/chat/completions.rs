use crate::entity::{llm_usage_tracking, user};
use crate::{
    entity::bit,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    usage_accounting::{
        UsageInvocationSettlement, UsageInvocationStart, estimate_text_tokens,
        settle_usage_invocation, start_usage_invocation,
    },
};
use axum::{
    Extension, Json,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue},
    response::Response as AxumResponse,
};
use flow_like::bit::Bit;
use flow_like_types::Bytes;
use flow_like_types::create_id;
use flow_like_types::{anyhow, json::json};
use futures_util::StreamExt;
use sea_orm::EntityTrait;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::Value as JsonValue;
use std::convert::Infallible;

const APP_ID_HEADER: &str = "x-flow-like-app-id";

#[derive(Debug, Clone, PartialEq)]
enum HostedProvider {
    OpenRouter,
    OpenAI,
    Anthropic,
    Bedrock,
    Azure,
    Vertex,
}

impl HostedProvider {
    fn from_provider_name(name: &str) -> Option<Self> {
        let name_lower = name.to_lowercase();
        match name_lower.as_str() {
            "hosted" | "hosted:openrouter" => Some(Self::OpenRouter),
            "hosted:openai" => Some(Self::OpenAI),
            "hosted:anthropic" => Some(Self::Anthropic),
            "hosted:bedrock" => Some(Self::Bedrock),
            "hosted:azure" => Some(Self::Azure),
            "hosted:vertex" => Some(Self::Vertex),
            _ => None,
        }
    }

    fn env_endpoint_key(&self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_ENDPOINT",
            Self::OpenAI => "HOSTED_OPENAI_ENDPOINT",
            Self::Anthropic => "HOSTED_ANTHROPIC_ENDPOINT",
            Self::Bedrock => "HOSTED_BEDROCK_ENDPOINT",
            Self::Azure => "HOSTED_AZURE_ENDPOINT",
            Self::Vertex => "HOSTED_VERTEX_ENDPOINT",
        }
    }

    fn env_api_key(&self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::OpenAI => "HOSTED_OPENAI_API_KEY",
            Self::Anthropic => "HOSTED_ANTHROPIC_API_KEY",
            Self::Bedrock => "HOSTED_BEDROCK_API_KEY",
            Self::Azure => "HOSTED_AZURE_API_KEY",
            Self::Vertex => "HOSTED_VERTEX_API_KEY",
        }
    }

    fn default_endpoint(&self) -> Option<&'static str> {
        match self {
            Self::OpenRouter => Some("https://openrouter.ai/api"),
            Self::OpenAI => Some("https://api.openai.com"),
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Bedrock => None,
            Self::Azure => None,
            Self::Vertex => None,
        }
    }

    fn completions_path(&self) -> &'static str {
        match self {
            Self::OpenRouter | Self::OpenAI | Self::Azure | Self::Bedrock | Self::Vertex => {
                "/v1/chat/completions"
            }
            Self::Anthropic => "/v1/messages",
        }
    }

    fn auth_header_name(&self) -> &'static str {
        match self {
            Self::Anthropic => "x-api-key",
            _ => "Authorization",
        }
    }

    fn uses_bearer_auth(&self) -> bool {
        !matches!(self, Self::Anthropic)
    }

    fn label(&self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Bedrock => "bedrock",
            Self::Azure => "azure",
            Self::Vertex => "vertex",
        }
    }
}

#[derive(Clone, Debug, Default)]
struct UsageRequestContext {
    app_id: Option<String>,
    user_id: String,
    technical_user_id: Option<String>,
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|header| header.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn resolve_usage_context(
    state: &AppState,
    user: &AppUser,
    headers: &HeaderMap,
) -> Result<UsageRequestContext, ApiError> {
    if let AppUser::Executor(executor) = user {
        return Ok(UsageRequestContext {
            app_id: Some(executor.app_id.clone()),
            user_id: executor.sub.clone(),
            technical_user_id: executor.technical_user_id.clone(),
        });
    }

    if let AppUser::APIKey(api_key) = user {
        user.execution_app_permission(&api_key.app_id, state)
            .await?;
        return Ok(UsageRequestContext {
            app_id: Some(api_key.app_id.clone()),
            user_id: user.effective_user_id()?,
            technical_user_id: Some(api_key.key_id.clone()),
        });
    }

    let app_id = header_string(headers, APP_ID_HEADER);
    if let Some(app_id) = app_id.as_deref() {
        user.execution_app_permission(app_id, state).await?;
    }

    Ok(UsageRequestContext {
        app_id,
        user_id: user.effective_user_id()?,
        technical_user_id: None,
    })
}

// --- helpers ---
async fn fetch_provider(
    state: &AppState,
    model_field: &str,
) -> Result<
    (
        flow_like::flow_like_model_provider::provider::ModelProvider,
        HostedProvider,
    ),
    ApiError,
> {
    let bit_model = bit::Entity::find_by_id(model_field)
        .one(&state.db)
        .await?
        .ok_or_else(|| anyhow!("Bit not found"))?;
    let bit_model = Bit::from(bit_model);
    let provider = bit_model
        .try_to_provider()
        .ok_or_else(|| anyhow!("Bit is not a model provider"))?;

    let hosted_provider = HostedProvider::from_provider_name(&provider.provider_name).ok_or_else(
        || {
            ApiError::bad_request(format!(
                "Unsupported provider: {}. Supported: Hosted, hosted:openrouter, hosted:openai, hosted:anthropic, hosted:bedrock, hosted:azure, hosted:vertex",
                provider.provider_name
            ))
        },
    )?;

    Ok((provider, hosted_provider))
}

async fn enforce_tier(
    user: &AppUser,
    state: &AppState,
    provider: &flow_like::flow_like_model_provider::provider::ModelProvider,
) -> Result<(), ApiError> {
    let user_tier: flow_like::hub::UserTier = user.tier(state).await?;
    let params = provider.params.clone().unwrap_or_default();
    let tier = params
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or("ENTERPRISE");
    if !user_tier.llm_tiers.iter().any(|t| t == tier) {
        tracing::warn!(
            "User tier {:?} does not allow access to model tier {}",
            user_tier,
            tier
        );
        return Err(ApiError::payment_required(format!(
            "This model requires the {} tier, which is not included in your plan.",
            tier
        )));
    }
    Ok(())
}

fn deduplicate_tools(body: &mut serde_json::Value) {
    if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        let mut seen_names = std::collections::HashSet::new();
        tools.retain(|tool| {
            let name = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            if name.is_empty() {
                true
            } else {
                seen_names.insert(name.to_string())
            }
        });
    }
}

fn ensure_user_first_message(body: &mut serde_json::Value) {
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        let first_non_system_idx = messages
            .iter()
            .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"));

        if let Some(idx) = first_non_system_idx {
            let role = messages[idx]
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if role == "assistant" {
                messages.insert(idx, json!({"role": "user", "content": ""}));
            }
        }
    }
}

fn enable_stream_usage_options(obj: &mut serde_json::Map<String, serde_json::Value>) {
    let stream_options = obj
        .entry("stream_options".to_string())
        .or_insert_with(|| json!({}));
    if !stream_options.is_object() {
        *stream_options = json!({});
    }
    if let Some(stream_options) = stream_options.as_object_mut() {
        stream_options.insert("include_usage".to_string(), json!(true));
    }
}

fn prepare_upstream_body(
    payload: &serde_json::Value,
    upstream_model_id: &str,
    tracking_user: Option<&str>,
    hosted_provider: &HostedProvider,
) -> (serde_json::Value, bool) {
    let mut body = payload.clone();
    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(obj) = body.as_object_mut() {
        obj.insert("model".to_string(), json!(upstream_model_id));

        match hosted_provider {
            HostedProvider::OpenRouter => {
                let usage = obj.entry("usage").or_insert_with(|| json!({}));
                if usage.is_object() {
                    usage
                        .as_object_mut()
                        .unwrap()
                        .insert("include".to_string(), json!(true));
                }
            }
            HostedProvider::Anthropic => {
                obj.insert("max_tokens".to_string(), json!(4096));
            }
            HostedProvider::OpenAI | HostedProvider::Azure => {
                if stream {
                    enable_stream_usage_options(obj);
                }
            }
            HostedProvider::Bedrock | HostedProvider::Vertex => {}
        }

        if let Some(u) = tracking_user {
            obj.insert("user".to_string(), json!(u));
        }
    }

    deduplicate_tools(&mut body);
    ensure_user_first_message(&mut body);

    (body, stream)
}

async fn build_provider_url(
    state: &AppState,
    hosted_provider: &HostedProvider,
) -> Result<(String, String), ApiError> {
    use flow_like_secrets::{ExposeSecret, SecretRef};

    let endpoint_key = hosted_provider.env_endpoint_key();
    let api_key_key = hosted_provider.env_api_key();

    let endpoint = state
        .secrets
        .get_secret_string(&SecretRef::new(endpoint_key))
        .await
        .ok()
        .map(|s| s.expose_secret().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| hosted_provider.default_endpoint().map(String::from))
        .ok_or_else(|| ApiError::internal(format!("{} not configured", endpoint_key)))?;

    let api_key = state
        .secrets
        .get_secret_string(&SecretRef::new(api_key_key))
        .await
        .map(|s| s.expose_secret().to_string())
        .unwrap_or_default();
    if api_key.is_empty() {
        return Err(ApiError::internal(format!(
            "{} not configured",
            api_key_key
        )));
    }

    let url = format!(
        "{}{}",
        endpoint.trim_end_matches('/'),
        hosted_provider.completions_path()
    );
    Ok((url, api_key))
}

// Accumulator for streaming usage/cost extraction
#[derive(Default, Debug)]
struct StreamingAccum {
    in_tok: Option<i64>,
    out_tok: Option<i64>,
    cost_micro: Option<i64>,
    provider_request_id: Option<String>,
    raw_usage: Option<JsonValue>,
}

#[derive(Clone, Debug, Default)]
struct ProviderUsageSnapshot {
    in_tok: Option<i64>,
    out_tok: Option<i64>,
    cost_micro: Option<i64>,
    provider_request_id: Option<String>,
    raw_usage: Option<JsonValue>,
}

fn extract_usage_and_cost_from_json(v: &serde_json::Value) -> Option<ProviderUsageSnapshot> {
    let usage = v.get("usage")?;
    let in_tok = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_i64());
    let out_tok = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|v| v.as_i64());
    let cost_micro = usage
        .get("cost_micro_dollars")
        .or_else(|| usage.get("cost_micros"))
        .and_then(|c| c.as_i64())
        .or_else(|| {
            usage
                .get("cost")
                .or_else(|| usage.get("total_cost"))
                .and_then(|c| c.as_f64())
                .map(|f| (f * 1_000_000.0) as i64)
        });
    let provider_request_id = v
        .get("id")
        .or_else(|| v.get("request_id"))
        .and_then(|id| id.as_str())
        .map(ToOwned::to_owned);
    if in_tok.is_some()
        || out_tok.is_some()
        || cost_micro.is_some()
        || provider_request_id.is_some()
    {
        Some(ProviderUsageSnapshot {
            in_tok,
            out_tok,
            cost_micro,
            provider_request_id,
            raw_usage: Some(usage.clone()),
        })
    } else {
        None
    }
}

fn estimate_chat_tokens(body: &JsonValue) -> i64 {
    fn collect_string_tokens(value: &JsonValue) -> i64 {
        match value {
            JsonValue::String(text) => estimate_text_tokens(text),
            JsonValue::Array(items) => items.iter().map(collect_string_tokens).sum(),
            JsonValue::Object(map) => map.values().map(collect_string_tokens).sum(),
            _ => 0,
        }
    }

    let prompt_tokens = collect_string_tokens(body);
    let max_output_tokens = body
        .get("max_tokens")
        .or_else(|| body.get("max_completion_tokens"))
        .and_then(|value| value.as_i64())
        .unwrap_or(1024);
    (prompt_tokens + max_output_tokens).max(1)
}

fn update_accum_from_snapshot(
    accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>,
    snapshot: ProviderUsageSnapshot,
) {
    let mut a = accum.lock().unwrap();
    a.in_tok = snapshot.in_tok.or(a.in_tok);
    a.out_tok = snapshot.out_tok.or(a.out_tok);
    a.cost_micro = snapshot.cost_micro.or(a.cost_micro);
    a.provider_request_id = snapshot
        .provider_request_id
        .or(a.provider_request_id.take());
    a.raw_usage = snapshot.raw_usage.or(a.raw_usage.take());
}

fn process_sse_line(accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>, line: &str) {
    let line = line.trim();
    if !line.starts_with("data: ") {
        return;
    }
    let data = &line[6..];
    if data == "[DONE]" {
        return;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data)
        && let Some(snapshot) = extract_usage_and_cost_from_json(&json)
    {
        update_accum_from_snapshot(accum, snapshot);
    }
}

/// Side-channel usage parser. `buffer` persists across stream chunks so a
/// `data:` line split across two network reads is reassembled before parsing;
/// only lines terminated by `\n` are parsed until `flush` drains the tail at
/// end-of-stream. Never touches the client-visible forwarded bytes.
fn parse_sse_bytes(
    accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>,
    buffer: &mut Vec<u8>,
    chunk: &[u8],
    flush: bool,
) {
    buffer.extend_from_slice(chunk);
    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=pos).collect();
        if let Ok(text) = std::str::from_utf8(&line) {
            process_sse_line(accum, text);
        }
    }
    if flush && !buffer.is_empty() {
        if let Ok(text) = std::str::from_utf8(buffer) {
            process_sse_line(accum, text);
        }
        buffer.clear();
    }
}

async fn finalize_llm_usage(
    state: &AppState,
    user_sub: &str,
    model_id: &str,
    usage_context: &UsageRequestContext,
    provider: &str,
    endpoint: &str,
    invocation_id: Option<&str>,
    accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>,
    latency_ms: f64,
) {
    let (in_tok, out_tok, cost_micro, provider_request_id, raw_usage) = {
        let a = accum.lock().unwrap();
        (
            a.in_tok,
            a.out_tok,
            a.cost_micro,
            a.provider_request_id.clone(),
            a.raw_usage.clone(),
        )
    };

    if in_tok.is_none() && out_tok.is_none() && cost_micro.is_none() {
        if let Err(e) = settle_usage_invocation(
            &state.db,
            invocation_id,
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
                latency_ms: Some(latency_ms),
                ..Default::default()
            },
        )
        .await
        {
            tracing::warn!(error=%e, "Failed to settle unknown LLM usage");
        }
        return;
    }

    if let Err(e) = track_llm_usage(
        state,
        user_sub,
        model_id,
        in_tok.unwrap_or(0),
        out_tok.unwrap_or(0),
        cost_micro.unwrap_or(0),
        latency_ms,
        usage_context.app_id.as_deref(),
        usage_context.technical_user_id.as_deref(),
        Some(provider),
        Some(endpoint),
        invocation_id,
        provider_request_id.as_deref(),
        raw_usage,
        crate::usage_accounting::STATUS_COMPLETED,
    )
    .await
    {
        tracing::warn!(error=%e, "Failed to track LLM usage");
    }
}

async fn finalize_cancelled_llm_usage(
    state: &AppState,
    user_sub: &str,
    model_id: &str,
    usage_context: &UsageRequestContext,
    provider: &str,
    endpoint: &str,
    invocation_id: Option<&str>,
    accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>,
    latency_ms: f64,
) {
    let (in_tok, out_tok, cost_micro, provider_request_id, raw_usage) = {
        let a = accum.lock().unwrap();
        (
            a.in_tok,
            a.out_tok,
            a.cost_micro,
            a.provider_request_id.clone(),
            a.raw_usage.clone(),
        )
    };

    if in_tok.is_none() && out_tok.is_none() && cost_micro.is_none() {
        if let Err(e) = settle_usage_invocation(
            &state.db,
            invocation_id,
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_CANCELLED,
                latency_ms: Some(latency_ms),
                error: Some("Client disconnected before streaming response completed".to_string()),
                ..Default::default()
            },
        )
        .await
        {
            tracing::warn!(error=%e, "Failed to settle cancelled LLM usage");
        }
        return;
    }

    if let Err(e) = track_llm_usage(
        state,
        user_sub,
        model_id,
        in_tok.unwrap_or(0),
        out_tok.unwrap_or(0),
        cost_micro.unwrap_or(0),
        latency_ms,
        usage_context.app_id.as_deref(),
        usage_context.technical_user_id.as_deref(),
        Some(provider),
        Some(endpoint),
        invocation_id,
        provider_request_id.as_deref(),
        raw_usage,
        crate::usage_accounting::STATUS_CANCELLED,
    )
    .await
    {
        tracing::warn!(error=%e, "Failed to track cancelled LLM usage");
    }
}

async fn handle_streaming(
    request_builder: flow_like_types::reqwest::RequestBuilder,
    state: AppState,
    user_sub: String,
    model_id: String,
    usage_context: UsageRequestContext,
    provider: String,
    endpoint: String,
    invocation_id: Option<String>,
) -> Result<AxumResponse, ApiError> {
    let started_at = std::time::Instant::now();
    let resp = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error=%e, "Upstream streaming request failed");
            let _ = settle_usage_invocation(
                &state.db,
                invocation_id.as_deref(),
                UsageInvocationSettlement {
                    status: crate::usage_accounting::STATUS_FAILED,
                    error: Some(e.to_string()),
                    ..Default::default()
                },
            )
            .await;
            return Err(ApiError::internal_error(anyhow!(
                "Upstream request failed: {e}"
            )));
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        tracing::error!(status=%status, body=%text, "Upstream error");
        let _ = settle_usage_invocation(
            &state.db,
            invocation_id.as_deref(),
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_FAILED,
                error: Some(format!("Upstream error {status}: {text}")),
                ..Default::default()
            },
        )
        .await;
        return Err(ApiError::bad_request("Upstream error"));
    }

    let mut builder = AxumResponse::builder().status(resp.status());
    if let Some(ct) = resp.headers().get(axum::http::header::CONTENT_TYPE) {
        builder = builder.header(axum::http::header::CONTENT_TYPE, ct);
    } else {
        builder = builder.header(axum::http::header::CONTENT_TYPE, "text/event-stream");
    }

    let accum = std::sync::Arc::new(std::sync::Mutex::new(StreamingAccum::default()));

    if llm_stream_background_drain_enabled() {
        let (tx, mut rx) =
            flow_like_types::tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(16);
        let accum_task = accum.clone();
        let invocation_id_task = invocation_id.clone();

        flow_like_types::tokio::spawn(async move {
            let mut upstream = resp.bytes_stream();
            let mut client_disconnected = false;
            let mut sse_buf: Vec<u8> = Vec::new();

            while let Some(chunk) = upstream.next().await {
                match chunk {
                    Ok(chunk_bytes) => {
                        parse_sse_bytes(&accum_task, &mut sse_buf, &chunk_bytes, false);
                        if tx.send(Ok(chunk_bytes)).await.is_err() {
                            client_disconnected = true;
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::error!(error=%error, "Error reading upstream stream");
                        if tx.send(Ok(Bytes::from_static(b""))).await.is_err() {
                            client_disconnected = true;
                        }
                        break;
                    }
                }
            }
            parse_sse_bytes(&accum_task, &mut sse_buf, &[], true);

            let latency_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            if client_disconnected {
                finalize_cancelled_llm_usage(
                    &state,
                    &user_sub,
                    &model_id,
                    &usage_context,
                    &provider,
                    &endpoint,
                    invocation_id_task.as_deref(),
                    &accum_task,
                    latency_ms,
                )
                .await;
            } else {
                finalize_llm_usage(
                    &state,
                    &user_sub,
                    &model_id,
                    &usage_context,
                    &provider,
                    &endpoint,
                    invocation_id_task.as_deref(),
                    &accum_task,
                    latency_ms,
                )
                .await;
            }
        });

        let body_stream = async_stream::stream! {
            while let Some(item) = rx.recv().await {
                yield item;
            }
        };
        let body = passthrough_byte_stream(body_stream);
        return Ok(builder.body(body).unwrap());
    }

    let accum_stream = accum.clone();
    let body_stream = async_stream::stream! {
        let mut upstream = resp.bytes_stream();
        let mut sse_buf: Vec<u8> = Vec::new();

        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk_bytes) => {
                    parse_sse_bytes(&accum_stream, &mut sse_buf, &chunk_bytes, false);
                    yield Ok(chunk_bytes);
                }
                Err(error) => {
                    tracing::error!(error=%error, "Error reading upstream stream");
                    yield Ok(Bytes::from_static(b""));
                    break;
                }
            }
        }
        parse_sse_bytes(&accum_stream, &mut sse_buf, &[], true);

        let latency_ms = started_at.elapsed().as_secs_f64() * 1000.0;
        finalize_llm_usage(
            &state,
            &user_sub,
            &model_id,
            &usage_context,
            &provider,
            &endpoint,
            invocation_id.as_deref(),
            &accum_stream,
            latency_ms,
        )
        .await;
    };
    let body = passthrough_byte_stream(body_stream);
    Ok(builder.body(body).unwrap())
}

fn llm_stream_background_drain_enabled() -> bool {
    if cfg!(feature = "lambda") {
        return false;
    }

    std::env::var("FLOWLIKE_LLM_STREAM_BACKGROUND_DRAIN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

async fn handle_non_streaming(
    request_builder: flow_like_types::reqwest::RequestBuilder,
    upstream_model_id: &str,
    state: &AppState,
    user_sub: &str,
    usage_context: &UsageRequestContext,
    provider: &str,
    endpoint: &str,
    invocation_id: Option<&str>,
) -> Result<AxumResponse, ApiError> {
    let start = std::time::Instant::now();
    let resp = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error=%e, "Upstream request failed");
            let _ = settle_usage_invocation(
                &state.db,
                invocation_id,
                UsageInvocationSettlement {
                    status: crate::usage_accounting::STATUS_FAILED,
                    error: Some(e.to_string()),
                    ..Default::default()
                },
            )
            .await;
            return Err(ApiError::internal_error(anyhow!(
                "Upstream request failed: {e}"
            )));
        }
    };
    let status = resp.status();
    let headers = resp.headers().clone();
    let body_bytes = resp.bytes().await.map_err(|e| {
        tracing::error!(error=%e, "Failed to read upstream body");
        anyhow!("Failed to read upstream body: {e}")
    })?;
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    if status.is_success() {
        tracing::info!(model = %upstream_model_id, bytes = body_bytes.len(), latency_ms = latency_ms, "LLM invoke success (non-stream)");
        if let Some(usage) = extract_usage_from_body(&body_bytes) {
            if let Err(e) = track_llm_usage(
                state,
                user_sub,
                upstream_model_id,
                usage.in_tok.unwrap_or(0),
                usage.out_tok.unwrap_or(0),
                usage.cost_micro.unwrap_or(0),
                latency_ms,
                usage_context.app_id.as_deref(),
                usage_context.technical_user_id.as_deref(),
                Some(provider),
                Some(endpoint),
                invocation_id,
                usage.provider_request_id.as_deref(),
                usage.raw_usage,
                crate::usage_accounting::STATUS_COMPLETED,
            )
            .await
            {
                tracing::warn!(error=%e, "Failed to track LLM usage");
            }
        } else if let Err(e) = settle_usage_invocation(
            &state.db,
            invocation_id,
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
                latency_ms: Some(latency_ms),
                ..Default::default()
            },
        )
        .await
        {
            tracing::warn!(error=%e, "Failed to settle unknown LLM usage");
        }
    } else {
        tracing::warn!(status = %status, body = %String::from_utf8_lossy(&body_bytes), "LLM invoke upstream error");
        let _ = settle_usage_invocation(
            &state.db,
            invocation_id,
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_FAILED,
                error: Some(format!(
                    "Upstream error {status}: {}",
                    String::from_utf8_lossy(&body_bytes)
                )),
                latency_ms: Some(latency_ms),
                ..Default::default()
            },
        )
        .await;
    }
    let mut out_headers = HeaderMap::new();
    if let Some(ct) = headers.get(axum::http::header::CONTENT_TYPE) {
        out_headers.insert(axum::http::header::CONTENT_TYPE, ct.clone());
    } else {
        out_headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }
    let response = AxumResponse::builder()
        .status(status)
        .body(Body::from(body_bytes))
        .unwrap();
    Ok(response)
}

#[utoipa::path(
    post,
    path = "/chat/completions",
    tag = "chat",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "LLM completion response (streaming or JSON)")
    )
)]
#[tracing::instrument(name = "POST /chat/completions", skip_all)]
pub async fn invoke_llm(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<AxumResponse, ApiError> {
    let model_field = payload
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("Missing 'model' field"))?;
    let (provider, hosted_provider) = fetch_provider(&state, model_field).await?;
    enforce_tier(&user, &state, &provider).await?;
    let usage_context = resolve_usage_context(&state, &user, &headers).await?;
    let upstream_model_id = provider
        .model_id
        .clone()
        .unwrap_or_else(|| model_field.to_string());
    let tracking_id_opt = user.tracking_id(&state).await.ok().flatten();
    let (upstream_body, stream) = prepare_upstream_body(
        &payload,
        &upstream_model_id,
        tracking_id_opt.as_deref(),
        &hosted_provider,
    );
    let (url, api_key) = build_provider_url(&state, &hosted_provider).await?;
    let provider_label = hosted_provider.label().to_string();
    let user_sub = usage_context.user_id.clone();
    let estimated_tokens = estimate_chat_tokens(&upstream_body);
    let invocation_id = start_usage_invocation(
        &state,
        UsageInvocationStart {
            kind: "llm",
            user_id: Some(&user_sub),
            technical_user_id: usage_context.technical_user_id.as_deref(),
            app_id: usage_context.app_id.as_deref(),
            provider: Some(&provider_label),
            endpoint: Some(&url),
            model_id: Some(&upstream_model_id),
            estimated_tokens,
            estimated_cost_micro_dollars: 0,
        },
    )
    .await?;
    let client = flow_like_types::reqwest::Client::new();

    let mut request_builder = if hosted_provider.uses_bearer_auth() {
        client.post(&url).bearer_auth(&api_key).json(&upstream_body)
    } else {
        client
            .post(&url)
            .header(hosted_provider.auth_header_name(), &api_key)
            .json(&upstream_body)
    };

    if hosted_provider == HostedProvider::OpenRouter {
        request_builder = request_builder
            .header("HTTP-Referer", "https://flow-like.com")
            .header("X-Title", "Flow-Like");
    }

    if hosted_provider == HostedProvider::Anthropic {
        request_builder = request_builder
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");
    }

    if let Some(tracking_id) = &tracking_id_opt {
        request_builder = request_builder.header("X-User-Id", tracking_id);
    }

    if stream {
        handle_streaming(
            request_builder,
            state,
            user_sub,
            upstream_model_id,
            usage_context,
            provider_label,
            url,
            invocation_id,
        )
        .await
    } else {
        handle_non_streaming(
            request_builder,
            &upstream_model_id,
            &state,
            &user_sub,
            &usage_context,
            &provider_label,
            &url,
            invocation_id.as_deref(),
        )
        .await
    }
}

// -------- Cost Tracking --------
fn extract_usage_from_body(body: &[u8]) -> Option<ProviderUsageSnapshot> {
    if let Ok(v) = serde_json::from_slice::<JsonValue>(body) {
        return extract_usage_and_cost_from_json(&v);
    }
    None
}

async fn track_llm_usage(
    state: &AppState,
    user_sub: &str,
    model: &str,
    token_in: i64,
    token_out: i64,
    price: i64,
    latency_ms: f64,
    app_id: Option<&str>,
    technical_user_id: Option<&str>,
    provider: Option<&str>,
    endpoint: Option<&str>,
    invocation_id: Option<&str>,
    provider_request_id: Option<&str>,
    raw_usage: Option<JsonValue>,
    settlement_status: &'static str,
) -> Result<(), flow_like_types::Error> {
    use chrono::Utc;
    use llm_usage_tracking::ActiveModel;
    let now = Utc::now().naive_utc();
    let record = ActiveModel {
        id: Set(create_id()),
        model_id: Set(model.to_string()),
        provider: Set(provider.map(ToOwned::to_owned)),
        endpoint: Set(endpoint.map(ToOwned::to_owned)),
        invocation_id: Set(invocation_id.map(ToOwned::to_owned)),
        provider_request_id: Set(provider_request_id.map(ToOwned::to_owned)),
        raw_usage: Set(raw_usage.clone()),
        token_in: Set(token_in),
        token_out: Set(token_out),
        latency: Set(Some(latency_ms)),
        user_id: Set(Some(user_sub.to_string())),
        technical_user_id: Set(technical_user_id.map(ToOwned::to_owned)),
        app_id: Set(app_id.map(ToOwned::to_owned)),
        price: Set(price),
        created_at: Set(now),
        updated_at: Set(now),
    };
    // Best-effort insert
    record.insert(&state.db).await?;

    settle_usage_invocation(
        &state.db,
        invocation_id,
        UsageInvocationSettlement {
            status: settlement_status,
            input_tokens: token_in,
            output_tokens: token_out,
            cost_micro_dollars: price,
            latency_ms: Some(latency_ms),
            provider_request_id: provider_request_id.map(ToOwned::to_owned),
            raw_usage,
            ..Default::default()
        },
    )
    .await?;

    if price != 0
        && let Some(existing) = user::Entity::find_by_id(user_sub).one(&state.db).await?
    {
        let total_llm_price = existing.total_llm_price.saturating_add(price);
        let mut active: user::ActiveModel = existing.into();
        active.total_llm_price = Set(total_llm_price);
        active.updated_at = Set(now);
        active.update(&state.db).await?;
    }

    Ok(())
}

// Turn a stream of Bytes into a Body verbatim.
fn passthrough_byte_stream<S>(s: S) -> Body
where
    S: futures_util::Stream<Item = Result<Bytes, Infallible>> + Send + 'static,
{
    Body::from_stream(s)
}

// -------- Tests --------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_upstream_body_rewrites_model_openrouter() {
        let payload = serde_json::json!({"model": "bit_123", "messages": [], "stream": false});
        let (rewritten, stream) = prepare_upstream_body(
            &payload,
            "gpt-4o-mini",
            Some("user_123"),
            &HostedProvider::OpenRouter,
        );
        assert!(!stream);
        assert_eq!(
            rewritten.get("model").unwrap().as_str().unwrap(),
            "gpt-4o-mini"
        );
        assert_eq!(rewritten.get("user").unwrap().as_str().unwrap(), "user_123");
        assert_eq!(
            rewritten
                .get("usage")
                .unwrap()
                .get("include")
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_prepare_upstream_body_rewrites_model_openai() {
        let payload = serde_json::json!({"model": "bit_123", "messages": [], "stream": false});
        let (rewritten, stream) = prepare_upstream_body(
            &payload,
            "gpt-4o",
            Some("user_123"),
            &HostedProvider::OpenAI,
        );
        assert!(!stream);
        assert_eq!(rewritten.get("model").unwrap().as_str().unwrap(), "gpt-4o");
        assert!(rewritten.get("usage").is_none());
        assert!(rewritten.get("stream_options").is_none());
    }

    #[test]
    fn test_prepare_upstream_body_enables_openai_compatible_stream_usage() {
        let payload = serde_json::json!({
            "model": "bit_123",
            "messages": [],
            "stream": true,
            "stream_options": {
                "include_obfuscation": false
            }
        });

        for hosted_provider in [HostedProvider::OpenAI, HostedProvider::Azure] {
            let (rewritten, stream) =
                prepare_upstream_body(&payload, "gpt-4o", None, &hosted_provider);
            assert!(stream);
            let stream_options = rewritten.get("stream_options").unwrap();
            assert_eq!(
                stream_options
                    .get("include_usage")
                    .and_then(|v| v.as_bool()),
                Some(true)
            );
            assert_eq!(
                stream_options
                    .get("include_obfuscation")
                    .and_then(|v| v.as_bool()),
                Some(false)
            );
        }
    }

    #[test]
    fn test_prepare_upstream_body_anthropic_adds_max_tokens() {
        let payload = serde_json::json!({"model": "bit_123", "messages": [], "stream": false});
        let (rewritten, _) =
            prepare_upstream_body(&payload, "claude-3-opus", None, &HostedProvider::Anthropic);
        assert_eq!(rewritten.get("max_tokens").unwrap().as_i64().unwrap(), 4096);
    }

    #[test]
    fn test_hosted_provider_from_name() {
        assert_eq!(
            HostedProvider::from_provider_name("Hosted"),
            Some(HostedProvider::OpenRouter)
        );
        assert_eq!(
            HostedProvider::from_provider_name("hosted:openrouter"),
            Some(HostedProvider::OpenRouter)
        );
        assert_eq!(
            HostedProvider::from_provider_name("hosted:openai"),
            Some(HostedProvider::OpenAI)
        );
        assert_eq!(
            HostedProvider::from_provider_name("hosted:anthropic"),
            Some(HostedProvider::Anthropic)
        );
        assert_eq!(
            HostedProvider::from_provider_name("hosted:bedrock"),
            Some(HostedProvider::Bedrock)
        );
        assert_eq!(
            HostedProvider::from_provider_name("hosted:azure"),
            Some(HostedProvider::Azure)
        );
        assert_eq!(
            HostedProvider::from_provider_name("hosted:vertex"),
            Some(HostedProvider::Vertex)
        );
        assert_eq!(HostedProvider::from_provider_name("unknown"), None);
    }

    #[test]
    fn test_extract_usage_from_body() {
        let body = serde_json::json!({
            "id": "chatcmpl-test",
            "usage": {"prompt_tokens": 12, "completion_tokens": 34, "total_tokens": 46}
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let usage = extract_usage_from_body(&bytes).unwrap();
        assert_eq!(usage.in_tok, Some(12));
        assert_eq!(usage.out_tok, Some(34));
        assert_eq!(usage.cost_micro, None);
        assert_eq!(usage.provider_request_id.as_deref(), Some("chatcmpl-test"));
    }

    #[test]
    fn test_deduplicate_tools() {
        let mut body = serde_json::json!({
            "tools": [
                {"function": {"name": "query_knowledge"}},
                {"function": {"name": "search"}},
                {"function": {"name": "query_knowledge"}},
                {"function": {"name": "search"}}
            ]
        });
        deduplicate_tools(&mut body);
        let tools = body.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_ensure_user_first_message() {
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "assistant", "content": "Hello!"}
            ]
        });
        ensure_user_first_message(&mut body);
        let messages = body.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].get("role").unwrap().as_str().unwrap(), "user");
    }

    #[test]
    fn test_ensure_user_first_message_already_valid() {
        let mut body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hi"}
            ]
        });
        ensure_user_first_message(&mut body);
        let messages = body.get("messages").unwrap().as_array().unwrap();
        assert_eq!(messages.len(), 2);
    }
}
