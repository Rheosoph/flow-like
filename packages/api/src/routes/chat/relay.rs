#![allow(clippy::too_many_arguments)]

//! Shared plumbing for the hosted model proxy.
//!
//! `/chat/completions` and `/responses` differ only in the request shape they
//! relay and the upstream path they target. Provider resolution, tier
//! enforcement, streaming passthrough and usage accounting are identical, and
//! live here so both routes settle invocations the same way.

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
    body::Body,
    http::{HeaderMap, HeaderValue},
    response::Response as AxumResponse,
};
use flow_like::bit::Bit;
use flow_like::flow_like_model_provider::provider::{ModelApiSurface, ModelProvider};
use flow_like_types::Bytes;
use flow_like_types::anyhow;
use flow_like_types::create_id;
use futures_util::StreamExt;
use sea_orm::EntityTrait;
use sea_orm::{ActiveModelTrait, Set};
use serde_json::Value as JsonValue;
use std::convert::Infallible;

const APP_ID_HEADER: &str = "x-flow-like-app-id";

#[derive(Debug, Clone, PartialEq)]
pub(super) enum HostedProvider {
    OpenRouter,
    OpenAI,
    Anthropic,
    Bedrock,
    Azure,
    Vertex,
}

impl HostedProvider {
    pub(super) fn from_provider_name(name: &str) -> Option<Self> {
        let name_lower = name.trim().to_lowercase();
        match name_lower.as_str() {
            "premium" | "internal" | "hosted" | "hosted:openrouter" => Some(Self::OpenRouter),
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

    /// Upstream URL for `surface`.
    ///
    /// A configured endpoint may already carry either surface's path, so both
    /// are stripped before the provider-specific prefix is rebuilt. That keeps a
    /// deployment whose `HOSTED_*_ENDPOINT` points straight at
    /// `…/v1/chat/completions` working on `/responses` too.
    pub(super) fn endpoint_url(&self, endpoint: &str, surface: ModelApiSurface) -> String {
        let endpoint = endpoint.trim_end_matches('/');
        let endpoint = endpoint
            .strip_suffix("/chat/completions")
            .or_else(|| endpoint.strip_suffix("/responses"))
            .unwrap_or(endpoint);
        let path = surface_path(surface);

        if endpoint.ends_with("/v1") {
            return format!("{endpoint}/{path}");
        }

        match self {
            Self::Azure if endpoint.ends_with("/openai") => format!("{endpoint}/v1/{path}"),
            Self::Azure => format!("{endpoint}/openai/v1/{path}"),
            Self::Vertex if endpoint.ends_with("/openapi") => format!("{endpoint}/{path}"),
            Self::OpenRouter | Self::OpenAI | Self::Anthropic | Self::Bedrock | Self::Vertex => {
                format!("{endpoint}/v1/{path}")
            }
        }
    }

    pub(super) fn label(&self) -> &'static str {
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

fn surface_path(surface: ModelApiSurface) -> &'static str {
    match surface {
        ModelApiSurface::ChatCompletions => "chat/completions",
        ModelApiSurface::Responses => "responses",
    }
}

fn surface_route(surface: ModelApiSurface) -> &'static str {
    match surface {
        ModelApiSurface::ChatCompletions => "/chat/completions",
        ModelApiSurface::Responses => "/responses",
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct UsageRequestContext {
    pub(super) app_id: Option<String>,
    pub(super) user_id: String,
    pub(super) technical_user_id: Option<String>,
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

async fn fetch_provider(
    state: &AppState,
    model_field: &str,
) -> Result<(ModelProvider, HostedProvider), ApiError> {
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
                "Unsupported provider: {}. Supported: Premium, Internal, Hosted, hosted:openrouter, hosted:openai, hosted:anthropic, hosted:bedrock, hosted:azure, hosted:vertex",
                provider.provider_name
            ))
        },
    )?;

    Ok((provider, hosted_provider))
}

async fn enforce_tier(
    user: &AppUser,
    state: &AppState,
    provider: &ModelProvider,
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

/// Drop repeated tool declarations, keeping the first of each name.
///
/// Chat Completions nests the name under `function`; the Responses API puts it
/// on the tool itself. Both shapes are accepted so the two routes share one
/// implementation.
pub(super) fn deduplicate_tools(body: &mut JsonValue) {
    if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        let mut seen_names = std::collections::HashSet::new();
        tools.retain(|tool| {
            let name = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| tool.get("name"))
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

async fn build_provider_url(
    state: &AppState,
    hosted_provider: &HostedProvider,
    surface: ModelApiSurface,
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

    let url = hosted_provider.endpoint_url(&endpoint, surface);
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
pub(super) struct ProviderUsageSnapshot {
    pub(super) in_tok: Option<i64>,
    pub(super) out_tok: Option<i64>,
    pub(super) cost_micro: Option<i64>,
    pub(super) provider_request_id: Option<String>,
    pub(super) raw_usage: Option<JsonValue>,
}

/// Locate the object carrying `usage`.
///
/// Chat Completions reports usage on the payload root. Responses reports it on
/// the `response` object, both in the final JSON body and on the streamed
/// `response.completed` event.
fn usage_container(v: &JsonValue) -> Option<&JsonValue> {
    if v.get("usage").is_some() {
        return Some(v);
    }
    v.get("response").filter(|r| r.get("usage").is_some())
}

pub(super) fn extract_usage_and_cost_from_json(v: &JsonValue) -> Option<ProviderUsageSnapshot> {
    let container = usage_container(v)?;
    let usage = container.get("usage")?;
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
    let provider_request_id = container
        .get("id")
        .or_else(|| container.get("request_id"))
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

pub(super) fn estimate_payload_tokens(body: &JsonValue) -> i64 {
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
        .or_else(|| body.get("max_output_tokens"))
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
    if let Ok(json) = serde_json::from_str::<JsonValue>(data)
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
                        // Ending the body silently here would make a truncated
                        // answer indistinguishable from a completed one — emit
                        // an in-band error frame the client can classify.
                        tracing::error!(error=%error, "Error reading upstream stream");
                        let frame = format!(
                            "data: {}\n\n",
                            flow_like_types::json::json!({
                                "error": {
                                    "message": format!("Upstream stream failed mid-response: {error}"),
                                    "type": "upstream_stream_error"
                                }
                            })
                        );
                        if tx.send(Ok(Bytes::from(frame))).await.is_err() {
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
                    // Ending the body silently here would make a truncated
                    // answer indistinguishable from a completed one — emit an
                    // in-band error frame the client can classify.
                    tracing::error!(error=%error, "Error reading upstream stream");
                    let frame = format!(
                        "data: {}\n\n",
                        flow_like_types::json::json!({
                            "error": {
                                "message": format!("Upstream stream failed mid-response: {error}"),
                                "type": "upstream_stream_error"
                            }
                        })
                    );
                    yield Ok(Bytes::from(frame));
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
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/json"));
    let response = AxumResponse::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .body(Body::from(body_bytes))
        .unwrap();
    Ok(response)
}

/// Rewrite a caller payload into the upstream request body.
///
/// Returns the body to forward and whether the caller asked for a stream.
pub(super) type PrepareUpstreamBody =
    fn(&JsonValue, &str, Option<&str>, &HostedProvider) -> (JsonValue, bool);

/// Resolve the Bit, authorize the caller, and relay the request upstream.
///
/// `surface` is the route's own surface. A Bit that declares a different one is
/// rejected rather than translated — the proxy forwards bytes, it does not
/// convert between the Chat Completions and Responses schemas.
pub(super) async fn relay_request(
    state: AppState,
    user: AppUser,
    headers: HeaderMap,
    payload: JsonValue,
    surface: ModelApiSurface,
    prepare_upstream_body: PrepareUpstreamBody,
) -> Result<AxumResponse, ApiError> {
    let model_field = payload
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("Missing 'model' field"))?;
    let (provider, hosted_provider) = fetch_provider(&state, model_field).await?;

    let bit_surface = provider.api_surface_or_default();
    if bit_surface != surface {
        return Err(ApiError::bad_request(format!(
            "Model {model_field} speaks the {} API; call {} instead.",
            bit_surface.as_str(),
            surface_route(bit_surface)
        )));
    }

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
    let (url, api_key) = build_provider_url(&state, &hosted_provider, surface).await?;
    let provider_label = hosted_provider.label().to_string();
    let user_sub = usage_context.user_id.clone();
    let estimated_tokens = estimate_payload_tokens(&upstream_body);
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

    let mut request_builder = client.post(&url).bearer_auth(&api_key).json(&upstream_body);

    if hosted_provider == HostedProvider::OpenRouter {
        request_builder = request_builder
            .header("HTTP-Referer", "https://flow-like.com")
            .header("X-Title", "Flow-Like");
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
pub(super) fn extract_usage_from_body(body: &[u8]) -> Option<ProviderUsageSnapshot> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hosted_provider_completion_urls_match_openai_compatible_endpoints() {
        let surface = ModelApiSurface::ChatCompletions;
        assert_eq!(
            HostedProvider::Anthropic.endpoint_url("https://api.anthropic.com", surface),
            "https://api.anthropic.com/v1/chat/completions"
        );
        assert_eq!(
            HostedProvider::Bedrock
                .endpoint_url("https://bedrock-mantle.eu-central-1.api.aws/v1", surface),
            "https://bedrock-mantle.eu-central-1.api.aws/v1/chat/completions"
        );
        assert_eq!(
            HostedProvider::Azure.endpoint_url("https://example.openai.azure.com", surface),
            "https://example.openai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            HostedProvider::Vertex.endpoint_url(
                "https://europe-west1-aiplatform.googleapis.com/v1/projects/project/locations/europe-west1/endpoints/openapi",
                surface,
            ),
            "https://europe-west1-aiplatform.googleapis.com/v1/projects/project/locations/europe-west1/endpoints/openapi/chat/completions"
        );
        assert_eq!(
            HostedProvider::OpenAI
                .endpoint_url("https://gateway.example/v1/chat/completions/", surface),
            "https://gateway.example/v1/chat/completions"
        );
    }

    #[test]
    fn test_hosted_provider_responses_urls() {
        let surface = ModelApiSurface::Responses;
        assert_eq!(
            HostedProvider::OpenAI.endpoint_url("https://api.openai.com", surface),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            HostedProvider::Azure.endpoint_url("https://example.openai.azure.com", surface),
            "https://example.openai.azure.com/openai/v1/responses"
        );
        assert_eq!(
            HostedProvider::OpenAI.endpoint_url("https://gateway.example/v1/responses/", surface),
            "https://gateway.example/v1/responses"
        );
        // A deployment pinned at the completions path still resolves /responses.
        assert_eq!(
            HostedProvider::OpenAI
                .endpoint_url("https://gateway.example/v1/chat/completions", surface),
            "https://gateway.example/v1/responses"
        );
    }

    #[test]
    fn test_hosted_provider_from_name() {
        assert_eq!(
            HostedProvider::from_provider_name("Hosted"),
            Some(HostedProvider::OpenRouter)
        );
        assert_eq!(
            HostedProvider::from_provider_name("Premium"),
            Some(HostedProvider::OpenRouter)
        );
        assert_eq!(
            HostedProvider::from_provider_name(" internal "),
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
    fn test_extract_usage_from_responses_body() {
        let body = serde_json::json!({
            "id": "resp_test",
            "object": "response",
            "usage": {"input_tokens": 7, "output_tokens": 11, "total_tokens": 18}
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let usage = extract_usage_from_body(&bytes).unwrap();
        assert_eq!(usage.in_tok, Some(7));
        assert_eq!(usage.out_tok, Some(11));
        assert_eq!(usage.provider_request_id.as_deref(), Some("resp_test"));
    }

    #[test]
    fn test_extract_usage_from_responses_completed_event() {
        let event = serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": "resp_stream",
                "usage": {"input_tokens": 3, "output_tokens": 5}
            }
        });
        let usage = extract_usage_and_cost_from_json(&event).unwrap();
        assert_eq!(usage.in_tok, Some(3));
        assert_eq!(usage.out_tok, Some(5));
        assert_eq!(usage.provider_request_id.as_deref(), Some("resp_stream"));
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
    fn test_deduplicate_flat_responses_tools() {
        let mut body = serde_json::json!({
            "tools": [
                {"type": "function", "name": "query_knowledge"},
                {"type": "function", "name": "search"},
                {"type": "function", "name": "query_knowledge"}
            ]
        });
        deduplicate_tools(&mut body);
        let tools = body.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_estimate_payload_tokens_uses_max_output_tokens() {
        let body = serde_json::json!({"input": "hello", "max_output_tokens": 256});
        assert!(estimate_payload_tokens(&body) > 256);
    }
}
