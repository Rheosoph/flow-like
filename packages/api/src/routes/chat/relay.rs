#![allow(clippy::too_many_arguments)]

//! Shared plumbing for the hosted model proxy.
//!
//! `/chat/completions` and `/responses` differ only in the request shape they
//! relay and the upstream path they target. Provider resolution, tier
//! enforcement, streaming passthrough and usage accounting are identical, and
//! live here so both routes settle invocations the same way.

use crate::entity::llm_usage_tracking;
use crate::{
    entity::bit,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    usage_accounting::{
        HostedRateSnapshot, UsageInvocationSettlement, UsageInvocationStart,
        configured_hosted_rate, record_provider_request_id, settle_hosted_usage_invocation,
        start_usage_invocation,
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
use sea_orm::Set;
use serde_json::Value as JsonValue;
use std::convert::Infallible;

const APP_ID_HEADER: &str = "x-flow-like-app-id";

static HOSTED_RATES: std::sync::LazyLock<moka::sync::Cache<String, HostedRateSnapshot>> =
    std::sync::LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(4096)
            .time_to_live(std::time::Duration::from_secs(300))
            .build()
    });

fn rate_from_catalog(model: &JsonValue) -> Result<HostedRateSnapshot, ApiError> {
    let price = |name: &str| -> Result<i64, ApiError> {
        let value = model
            .get("pricing")
            .and_then(|value| value.get(name))
            .and_then(|value| {
                value
                    .as_str()
                    .and_then(|value| value.parse::<f64>().ok())
                    .or_else(|| value.as_f64())
            })
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| {
                ApiError::internal(format!("Hosted model {name} price is unavailable"))
            })?;
        let micros = value
            * if name == "request" {
                1_000_000.0
            } else {
                1_000_000_000_000.0
            };
        if micros >= i64::MAX as f64 {
            return Err(ApiError::internal("Hosted model price is out of range"));
        }
        Ok(micros.ceil() as i64)
    };
    let rate = HostedRateSnapshot {
        version: format!(
            "openrouter-{}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%MZ")
        ),
        input_micro_usd_per_million_tokens: price("prompt")?,
        input_micro_usd_per_million_bytes: None,
        max_input_bytes: None,
        output_micro_usd_per_million_tokens: price("completion")?,
        request_micro_usd: price("request")?,
        context_tokens: model
            .get("context_length")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0),
        usd_micro_per_eur: 1_159_200,
        funding_basis_points: 550,
        api_micro_usd_per_million_ms: 20_001,
        serving_request_micro_usd: 1,
        max_request_ms: 240_000,
    };
    rate.validate()?;
    Ok(rate)
}

async fn hosted_rate(
    provider: &HostedProvider,
    model: &str,
    api_key: &str,
) -> Result<HostedRateSnapshot, ApiError> {
    if let Some(rate) = configured_hosted_rate(provider.label(), model)? {
        return Ok(rate);
    }
    if *provider != HostedProvider::OpenRouter {
        return Err(ApiError::internal("Hosted model pricing is not configured"));
    }
    if let Some(rate) = HOSTED_RATES.get(model) {
        return Ok(rate);
    }
    let response = flow_like_types::reqwest::Client::new()
        .get("https://openrouter.ai/api/v1/models")
        .bearer_auth(api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| ApiError::internal("Unable to verify hosted model pricing"))?
        .error_for_status()
        .map_err(|_| ApiError::internal("Unable to verify hosted model pricing"))?;
    let catalog: JsonValue = response
        .json()
        .await
        .map_err(|_| ApiError::internal("Hosted model pricing response is invalid"))?;
    for entry in catalog
        .get("data")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(id), Ok(rate)) = (
            entry.get("id").and_then(JsonValue::as_str),
            rate_from_catalog(entry),
        ) {
            HOSTED_RATES.insert(id.to_owned(), rate);
        }
    }
    HOSTED_RATES
        .get(model)
        .ok_or_else(|| ApiError::internal("Hosted model pricing is unavailable"))
}

/// Bound the provider work before reserving money. Hidden conversation state and
/// media inputs reserve the context ceiling because their tokens are not in the
/// request text. Hosted provider tools require their own tariff before enabling.
fn bound_hosted_request(
    body: &mut JsonValue,
    surface: ModelApiSurface,
    rate: &HostedRateSnapshot,
    provider: &HostedProvider,
) -> Result<(i64, i64), ApiError> {
    if rate.input_micro_usd_per_million_bytes.is_some() {
        return Err(ApiError::internal(
            "Chat models require a token-based provider tariff",
        ));
    }
    let obj = body
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("Expected a request object"))?;
    if obj.get("n").is_some_and(|n| n.as_i64() != Some(1))
        || obj.get("best_of").is_some_and(|n| n.as_i64() != Some(1))
        || obj.get("background").and_then(JsonValue::as_bool) == Some(true)
        || obj.get("audio").is_some()
        || obj.get("web_search_options").is_some()
        || obj.get("prediction").is_some()
        || obj
            .get("service_tier")
            .and_then(JsonValue::as_str)
            .is_some_and(|tier| !matches!(tier, "default" | "auto"))
        || obj
            .get("modalities")
            .and_then(JsonValue::as_array)
            .is_some_and(|modalities| modalities.iter().any(|mode| mode.as_str() != Some("text")))
        || obj
            .get("plugins")
            .and_then(JsonValue::as_array)
            .is_some_and(|plugins| !plugins.is_empty())
        || obj
            .get("tools")
            .and_then(JsonValue::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool.get("type")
                        .and_then(JsonValue::as_str)
                        .is_some_and(|kind| !matches!(kind, "function" | "custom"))
                })
            })
    {
        return Err(ApiError::bad_request(
            "Hosted AI supports one response and client-executed tools. Use your own provider for background generation or provider-billed tools.",
        ));
    }
    let output = obj
        .get("max_tokens")
        .or_else(|| obj.get("max_completion_tokens"))
        .or_else(|| obj.get("max_output_tokens"))
        .map(|value| {
            value.as_i64().filter(|value| *value > 0).ok_or_else(|| {
                ApiError::bad_request("Output token limit must be a positive integer")
            })
        })
        .transpose()?
        .unwrap_or(4096)
        .min(rate.context_tokens.saturating_sub(1));
    if output <= 0 {
        return Err(ApiError::bad_request(
            "Model context cannot fit this request",
        ));
    }
    for field in [
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "models",
        "route",
    ] {
        obj.remove(field);
    }
    obj.insert(
        match surface {
            ModelApiSurface::Responses => "max_output_tokens",
            _ => "max_completion_tokens",
        }
        .into(),
        serde_json::json!(output),
    );
    if *provider == HostedProvider::OpenRouter {
        let routing = obj
            .entry("provider")
            .or_insert_with(|| serde_json::json!({}));
        if !routing.is_object() {
            *routing = serde_json::json!({});
        }
        routing["max_price"] = serde_json::json!({
            "prompt": rate.input_micro_usd_per_million_tokens as f64 / 1_000_000.0,
            "completion": rate.output_micro_usd_per_million_tokens as f64 / 1_000_000.0,
        });
    }
    let serialized =
        serde_json::to_vec(body).map_err(|_| ApiError::bad_request("Unable to size request"))?;
    let hidden_input = body.get("previous_response_id").is_some()
        || body.get("conversation").is_some()
        || String::from_utf8_lossy(&serialized).contains("image")
        || String::from_utf8_lossy(&serialized).contains("audio")
        || String::from_utf8_lossy(&serialized).contains("file");
    let input = if hidden_input {
        rate.context_tokens.saturating_sub(output)
    } else {
        (serialized.len() as i64).saturating_add(1024)
    };
    if input.saturating_add(output) > rate.context_tokens {
        return Err(ApiError::bad_request(
            "Request exceeds this hosted model's reserved context capacity. Reduce the input or output token limit.",
        ));
    }
    Ok((
        input.saturating_add(output),
        rate.provider_cost(input, output),
    ))
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum HostedProvider {
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

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct UsageRequestContext {
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
    payer_id: &str,
    state: &AppState,
    provider: &ModelProvider,
) -> Result<(), ApiError> {
    let (plan, user_tier) = crate::quota::payer_plan(state, payer_id).await?;
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
        return Err(ApiError::hosted_model_unavailable(payer_id, &plan, tier));
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

pub(super) async fn build_provider_url(
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
    completed: bool,
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
    let container = usage_container(v).unwrap_or_else(|| v.get("response").unwrap_or(v));
    let empty_usage = JsonValue::Null;
    let usage = container.get("usage").unwrap_or(&empty_usage);
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
                .filter(|f| f.is_finite() && *f >= 0.0)
                .map(|f| (f * 1_000_000.0).ceil().min(i64::MAX as f64) as i64)
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
            raw_usage: (!usage.is_null()).then(|| usage.clone()),
        })
    } else {
        None
    }
}

#[cfg(test)]
pub(super) fn estimate_payload_tokens(body: &JsonValue) -> i64 {
    use crate::usage_accounting::estimate_text_tokens;
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
    let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
        return;
    };
    if data == "[DONE]" {
        accum.lock().unwrap().completed = true;
        return;
    }
    if let Ok(json) = serde_json::from_str::<JsonValue>(data)
        && let Some(snapshot) = extract_usage_and_cost_from_json(&json)
    {
        if json.get("type").and_then(JsonValue::as_str) == Some("response.completed") {
            accum.lock().unwrap().completed = true;
        }
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

async fn persist_stream_provider_id(
    state: &AppState,
    invocation_id: Option<&str>,
    accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>,
    saved: &mut Option<String>,
) {
    let id = accum.lock().unwrap().provider_request_id.clone();
    if id != *saved
        && let Some(id) = id
    {
        match record_provider_request_id(state, invocation_id, &id).await {
            Ok(()) => *saved = Some(id),
            Err(error) => tracing::warn!(%error, "Unable to persist hosted provider request ID"),
        }
    }
}

async fn resolved_provider_cost(
    state: &AppState,
    invocation_id: Option<&str>,
    input: Option<i64>,
    output: Option<i64>,
    reported_cost: Option<i64>,
) -> Option<i64> {
    if let Some(cost) = reported_cost.filter(|cost| *cost >= 0) {
        return Some(cost);
    }
    let (Some(input), Some(output), Some(id)) = (input, output, invocation_id) else {
        return None;
    };
    let row = crate::entity::usage_invocation::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .ok()??;
    let rate: HostedRateSnapshot =
        serde_json::from_value(row.raw_usage?.get("accounting")?.clone()).ok()?;
    Some(rate.provider_cost(input, output))
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

    let completed = accum.lock().unwrap().completed;
    let cost_micro = if completed {
        resolved_provider_cost(state, invocation_id, in_tok, out_tok, cost_micro).await
    } else {
        None
    };
    if cost_micro.is_none() {
        if let Err(e) = settle_hosted_usage_invocation(
            state,
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

pub(super) async fn handle_streaming(
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
            let _ = settle_hosted_usage_invocation(
                &state,
                invocation_id.as_deref(),
                UsageInvocationSettlement {
                    status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
                    error: Some(e.to_string()),
                    latency_ms: Some(started_at.elapsed().as_secs_f64() * 1000.0),
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
        let _ = settle_hosted_usage_invocation(
            &state,
            invocation_id.as_deref(),
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
                error: Some(format!("Upstream error {status}: {text}")),
                latency_ms: Some(started_at.elapsed().as_secs_f64() * 1000.0),
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
            let mut saved_provider_id = None;
            let mut sse_buf: Vec<u8> = Vec::new();

            while let Some(chunk) = upstream.next().await {
                match chunk {
                    Ok(chunk_bytes) => {
                        parse_sse_bytes(&accum_task, &mut sse_buf, &chunk_bytes, false);
                        persist_stream_provider_id(
                            &state,
                            invocation_id_task.as_deref(),
                            &accum_task,
                            &mut saved_provider_id,
                        )
                        .await;
                        if !client_disconnected && tx.send(Ok(chunk_bytes)).await.is_err() {
                            client_disconnected = true;
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
                        let _ = tx.send(Ok(Bytes::from(frame))).await;
                        break;
                    }
                }
            }
            parse_sse_bytes(&accum_task, &mut sse_buf, &[], true);

            let latency_ms = started_at.elapsed().as_secs_f64() * 1000.0;
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
        let mut saved_provider_id = None;

        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk_bytes) => {
                    parse_sse_bytes(&accum_stream, &mut sse_buf, &chunk_bytes, false);
                    persist_stream_provider_id(&state, invocation_id.as_deref(), &accum_stream, &mut saved_provider_id).await;
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
    !super::hosted_worker::enabled()
}

pub(super) async fn handle_non_streaming(
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
            let _ = settle_hosted_usage_invocation(
                &state,
                invocation_id,
                UsageInvocationSettlement {
                    status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
                    error: Some(e.to_string()),
                    latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
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
        let usage = extract_usage_from_body(&body_bytes);
        let cost = if let Some(usage) = &usage {
            resolved_provider_cost(
                state,
                invocation_id,
                usage.in_tok,
                usage.out_tok,
                usage.cost_micro,
            )
            .await
        } else {
            None
        };
        if let (Some(usage), Some(cost)) = (usage, cost) {
            if let Err(e) = track_llm_usage(
                state,
                user_sub,
                upstream_model_id,
                usage.in_tok.unwrap_or(0),
                usage.out_tok.unwrap_or(0),
                cost,
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
        } else if let Err(e) = settle_hosted_usage_invocation(
            state,
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
        let _ = settle_hosted_usage_invocation(
            state,
            invocation_id,
            UsageInvocationSettlement {
                status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
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

    let usage_context = resolve_usage_context(&state, &user, &headers).await?;
    let payer_id = crate::quota::resolve_payer(
        &state,
        Some(&usage_context.user_id),
        usage_context.app_id.as_deref(),
    )
    .await?;
    enforce_tier(&payer_id, &state, &provider).await?;
    let upstream_model_id = provider
        .model_id
        .clone()
        .unwrap_or_else(|| model_field.to_string());
    let tracking_id_opt = user.tracking_id(&state).await.ok().flatten();
    let (mut upstream_body, stream) = prepare_upstream_body(
        &payload,
        &upstream_model_id,
        tracking_id_opt.as_deref(),
        &hosted_provider,
    );
    let (url, api_key) = build_provider_url(&state, &hosted_provider, surface).await?;
    let provider_label = hosted_provider.label().to_string();
    let user_sub = usage_context.user_id.clone();
    let mut rate = hosted_rate(&hosted_provider, &upstream_model_id, &api_key).await?;
    super::hosted_worker::apply_worker_tariff(&mut rate);
    let (estimated_tokens, estimated_cost) =
        bound_hosted_request(&mut upstream_body, surface, &rate, &hosted_provider)?;
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
            estimated_cost_micro_dollars: estimated_cost,
            rate: Some(rate.clone()),
        },
    )
    .await?;
    let id = invocation_id
        .as_deref()
        .ok_or_else(|| ApiError::internal("Hosted AI reservation is missing"))?;
    if super::hosted_worker::enabled() {
        return super::hosted_worker::dispatch(
            state,
            super::hosted_worker::HostedAiJob {
                operation_id: id.to_owned(),
                request: super::hosted_worker::HostedWork::Chat {
                    body: upstream_body,
                    responses_api: surface == ModelApiSurface::Responses,
                    stream,
                    provider: hosted_provider,
                    model_id: upstream_model_id,
                    context: usage_context,
                },
                deadline: chrono::Utc::now() + chrono::Duration::milliseconds(rate.max_request_ms),
            },
        )
        .await;
    }
    if !crate::quota::mark_started(&state, id).await? {
        return Err(ApiError::conflict(
            "Hosted AI operation has already started",
        ));
    }
    let client = flow_like_types::reqwest::Client::new();

    let mut request_builder = client
        .post(&url)
        .bearer_auth(&api_key)
        .json(&upstream_body)
        .timeout(std::time::Duration::from_millis(rate.max_request_ms as u64));

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
    let now = Utc::now().fixed_offset();
    let record = ActiveModel {
        id: Set(invocation_id
            .map(ToOwned::to_owned)
            .unwrap_or_else(create_id)),
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
    settle_hosted_usage_invocation(
        state,
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

    llm_usage_tracking::Entity::insert(record)
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(llm_usage_tracking::Column::Id)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&state.db)
        .await?;

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

    fn test_rate() -> HostedRateSnapshot {
        rate_from_catalog(&serde_json::json!({
            "context_length": 32_768,
            "pricing": {"prompt":"0.000001","completion":"0.000004","request":"0"}
        }))
        .unwrap()
    }

    #[test]
    fn hosted_reservation_locks_model_routing_and_output_work() {
        let mut body = serde_json::json!({"model":"selected", "messages":[{"role":"user","content":"Hello"}],
            "models":["expensive-fallback"], "max_tokens":512, "max_output_tokens":999999,
            "provider":{"max_price":{"prompt":999}}});
        let (tokens, cost) = bound_hosted_request(
            &mut body,
            ModelApiSurface::ChatCompletions,
            &test_rate(),
            &HostedProvider::OpenRouter,
        )
        .unwrap();
        assert!(tokens > 512 && cost > 0);
        assert!(body.get("models").is_none());
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_output_tokens").is_none());
        assert_eq!(body["max_completion_tokens"], 512);
        assert_eq!(body["provider"]["max_price"]["prompt"], 1.0);
    }

    #[test]
    fn hosted_requests_reject_work_with_unbounded_extra_provider_charges() {
        for mut body in [
            serde_json::json!({"n":2}),
            serde_json::json!({"best_of":2}),
            serde_json::json!({"background":true}),
            serde_json::json!({"web_search_options":{}}),
            serde_json::json!({"prediction":{"content":"predicted tokens"}}),
            serde_json::json!({"service_tier":"priority"}),
            serde_json::json!({"tools":[{"type":"web_search"}]}),
            serde_json::json!({"modalities":["audio"]}),
        ] {
            assert!(
                bound_hosted_request(
                    &mut body,
                    ModelApiSurface::Responses,
                    &test_rate(),
                    &HostedProvider::OpenRouter
                )
                .is_err()
            );
        }
    }

    #[test]
    fn provider_receipt_is_available_before_stream_usage_arrives() {
        let accum = std::sync::Arc::new(std::sync::Mutex::new(StreamingAccum::default()));
        let mut buffer = Vec::new();
        parse_sse_bytes(
            &accum,
            &mut buffer,
            b"data:{\"id\":\"gen-receipt\",\"choices\":[]}\n\n",
            false,
        );
        assert_eq!(
            accum.lock().unwrap().provider_request_id.as_deref(),
            Some("gen-receipt")
        );
        assert!(!accum.lock().unwrap().completed);
        parse_sse_bytes(&accum, &mut buffer, b"data: [DONE]\n\n", false);
        assert!(accum.lock().unwrap().completed);
    }

    #[test]
    fn split_sse_usage_is_reassembled_and_missing_cost_stays_unknown() {
        let accum = std::sync::Arc::new(std::sync::Mutex::new(StreamingAccum::default()));
        let mut buffer = Vec::new();
        parse_sse_bytes(
            &accum,
            &mut buffer,
            b"data: {\"usage\":{\"prompt_tok",
            false,
        );
        parse_sse_bytes(
            &accum,
            &mut buffer,
            b"ens\":4,\"completion_tokens\":8}}\n",
            false,
        );
        let snapshot = accum.lock().unwrap();
        assert_eq!(snapshot.in_tok, Some(4));
        assert_eq!(snapshot.out_tok, Some(8));
        assert_eq!(snapshot.cost_micro, None);
        assert!(!snapshot.completed);
    }

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
