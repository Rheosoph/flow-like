use crate::entity::llm_usage_tracking;
use crate::{entity::bit, error::ApiError, middleware::jwt::AppUser, state::AppState};
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
        return Err(ApiError::FORBIDDEN);
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

fn prepare_upstream_body(
    payload: &serde_json::Value,
    upstream_model_id: &str,
    tracking_user: Option<&str>,
    hosted_provider: &HostedProvider,
) -> (serde_json::Value, bool) {
    let mut body = payload.clone();
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
            _ => {}
        }

        if let Some(u) = tracking_user {
            obj.insert("user".to_string(), json!(u));
        }
    }

    deduplicate_tools(&mut body);
    ensure_user_first_message(&mut body);

    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
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
}

fn extract_usage_and_cost_from_json(
    v: &serde_json::Value,
) -> Option<(Option<i64>, Option<i64>, Option<i64>)> {
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
        .get("cost")
        .or_else(|| usage.get("total_cost"))
        .and_then(|c| c.as_f64())
        .map(|f| (f * 1_000_000.0) as i64);
    if in_tok.is_some() || out_tok.is_some() || cost_micro.is_some() {
        Some((in_tok, out_tok, cost_micro))
    } else {
        None
    }
}

fn parse_sse_bytes(accum: &std::sync::Arc<std::sync::Mutex<StreamingAccum>>, bytes: &Bytes) {
    if let Ok(text) = std::str::from_utf8(bytes) {
        for line in text.split('\n') {
            let line = line.trim();
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                continue;
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data)
                && let Some((in_tok, out_tok, cost_micro)) = extract_usage_and_cost_from_json(&json)
                && (in_tok.is_some() || out_tok.is_some() || cost_micro.is_some())
            {
                let mut a = accum.lock().unwrap();
                a.in_tok = in_tok.or(a.in_tok);
                a.out_tok = out_tok.or(a.out_tok);
                a.cost_micro = cost_micro.or(a.cost_micro);
            }
        }
    }
}

use futures_util::Stream;
use futures_util::task::{Context, Poll};
use pin_project_lite::pin_project;
use std::pin::Pin;

pin_project! {
    struct TrackingStream<S> {
        #[pin]
        inner: S,
        accum: std::sync::Arc<std::sync::Mutex<StreamingAccum>>,
        finalized: bool,
        state: AppState,
        user: String,
        model: String,
        started_at: std::time::Instant,
    }
}

impl<S> Stream for TrackingStream<S>
where
    S: Stream<Item = Result<Bytes, flow_like_types::reqwest::Error>>,
{
    type Item = Result<Bytes, Infallible>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk_bytes))) => {
                parse_sse_bytes(this.accum, &chunk_bytes);
                Poll::Ready(Some(Ok(chunk_bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                tracing::error!(error=%e, "Error reading upstream stream");
                Poll::Ready(Some(Ok(Bytes::from_static(b""))))
            }
            Poll::Ready(None) => {
                if !*this.finalized {
                    *this.finalized = true;
                    let acc = this.accum.clone();
                    let state_c = this.state.clone();
                    let user_c = this.user.clone();
                    let model_c = this.model.clone();
                    let latency_ms = this.started_at.elapsed().as_secs_f64() * 1000.0;
                    flow_like_types::tokio::spawn(async move {
                        let (in_tok, out_tok, cost_micro) = {
                            let a = acc.lock().unwrap();
                            (a.in_tok, a.out_tok, a.cost_micro)
                        };
                        if let (Some(in_t), Some(out_t)) = (in_tok, out_tok) {
                            let price = cost_micro.unwrap_or(0);
                            if let Err(e) = track_llm_usage(
                                &state_c, &user_c, &model_c, in_t, out_t, price, latency_ms,
                            )
                            .await
                            {
                                tracing::warn!(error=%e, "Failed to track streaming LLM usage");
                            }
                        }
                    });
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

async fn handle_streaming(
    request_builder: flow_like_types::reqwest::RequestBuilder,
    state: AppState,
    user_sub: String,
    model_id: String,
) -> Result<AxumResponse, ApiError> {
    let resp = request_builder.send().await.map_err(|e| {
        tracing::error!(error=%e, "Upstream streaming request failed");
        anyhow!("Upstream request failed: {e}")
    })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        tracing::error!(status=%status, body=%text, "Upstream error");
        return Err(ApiError::bad_request("Upstream error"));
    }

    let mut builder = AxumResponse::builder().status(resp.status());
    if let Some(ct) = resp.headers().get(axum::http::header::CONTENT_TYPE) {
        builder = builder.header(axum::http::header::CONTENT_TYPE, ct);
    } else {
        builder = builder.header(axum::http::header::CONTENT_TYPE, "text/event-stream");
    }

    // Wrap upstream stream so we can observe chunks while leaving bytes unchanged.
    let upstream = resp.bytes_stream();
    let tracking_stream = TrackingStream {
        inner: upstream,
        accum: std::sync::Arc::new(std::sync::Mutex::new(StreamingAccum::default())),
        finalized: false,
        state,
        user: user_sub,
        model: model_id,
        started_at: std::time::Instant::now(),
    };
    let body_stream = tracking_stream.map(|res| res);
    let body = passthrough_byte_stream(body_stream);
    Ok(builder.body(body).unwrap())
}

async fn handle_non_streaming(
    request_builder: flow_like_types::reqwest::RequestBuilder,
    upstream_model_id: &str,
    state: &AppState,
    user_sub: &str,
) -> Result<AxumResponse, ApiError> {
    let start = std::time::Instant::now();
    let resp = request_builder.send().await.map_err(|e| {
        tracing::error!(error=%e, "Upstream request failed");
        anyhow!("Upstream request failed: {e}")
    })?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let body_bytes = resp.bytes().await.map_err(|e| {
        tracing::error!(error=%e, "Failed to read upstream body");
        anyhow!("Failed to read upstream body: {e}")
    })?;
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    if status.is_success() {
        tracing::info!(model = %upstream_model_id, bytes = body_bytes.len(), latency_ms = latency_ms, "LLM invoke success (non-stream)");
        if let Some((in_tok, out_tok)) = extract_usage_from_body(&body_bytes)
            && let Err(e) = track_llm_usage(
                state,
                user_sub,
                upstream_model_id,
                in_tok,
                out_tok,
                0,
                latency_ms,
            )
            .await
        {
            tracing::warn!(error=%e, "Failed to track LLM usage");
        }
    } else {
        tracing::warn!(status = %status, body = %String::from_utf8_lossy(&body_bytes), "LLM invoke upstream error");
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
#[tracing::instrument(name = "POST /chat/completions", skip(state, user, payload))]
pub async fn invoke_llm(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<serde_json::Value>,
) -> Result<AxumResponse, ApiError> {
    user.executor_scoped_sub()?;
    let model_field = payload
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("Missing 'model' field"))?;
    let (provider, hosted_provider) = fetch_provider(&state, model_field).await?;
    enforce_tier(&user, &state, &provider).await?;
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

    let user_sub = user.executor_scoped_sub()?;
    if stream {
        handle_streaming(request_builder, state, user_sub, upstream_model_id).await
    } else {
        handle_non_streaming(request_builder, &upstream_model_id, &state, &user_sub).await
    }
}

// -------- Cost Tracking --------
fn extract_usage_from_body(body: &[u8]) -> Option<(i64, i64)> {
    if let Ok(v) = serde_json::from_slice::<JsonValue>(body) {
        // Try common OpenAI style usage fields
        if let Some(usage) = v.get("usage") {
            let in_tok = usage
                .get("prompt_tokens")
                .or_else(|| usage.get("input_tokens"))?
                .as_i64()?;
            let out_tok = usage
                .get("completion_tokens")
                .or_else(|| usage.get("output_tokens"))?
                .as_i64()?;
            return Some((in_tok, out_tok));
        }
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
) -> Result<(), flow_like_types::Error> {
    use chrono::Utc;
    use llm_usage_tracking::ActiveModel;
    let now = Utc::now().naive_utc();
    let record = ActiveModel {
        id: Set(create_id()),
        model_id: Set(model.to_string()),
        token_in: Set(token_in),
        token_out: Set(token_out),
        latency: Set(Some(latency_ms)),
        user_id: Set(Some(user_sub.to_string())),
        app_id: Set(None),
        price: Set(price),
        created_at: Set(now),
        updated_at: Set(now),
    };
    // Best-effort insert
    match record.insert(&state.db).await {
        Ok(_) => Ok(()),
        Err(e) => Err(e.into()),
    }
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
        assert_eq!(usage, (12, 34));
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
