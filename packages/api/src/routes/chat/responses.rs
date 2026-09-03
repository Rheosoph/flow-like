//! OpenAI Responses API relay.
//!
//! Mounted at the router root (`/responses`, not `/chat/responses`) because
//! Rig's Responses client appends `/responses` to the base URL that hosted Bits
//! already point at (`{api}/api/v1`).

use super::relay::{HostedProvider, PrepareUpstreamBody, deduplicate_tools, relay_request};
use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State, http::HeaderMap, response::Response as AxumResponse};
use flow_like::flow_like_model_provider::provider::ModelApiSurface;
use flow_like_types::json::json;

/// Rewrite the caller payload for the upstream Responses endpoint.
///
/// Unlike Chat Completions there is no `stream_options` opt-in: the Responses
/// API always reports usage on the terminal `response.completed` event, and
/// rejects the parameter as unknown.
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

        if hosted_provider == &HostedProvider::OpenRouter {
            let usage = obj.entry("usage").or_insert_with(|| json!({}));
            if usage.is_object() {
                usage
                    .as_object_mut()
                    .unwrap()
                    .insert("include".to_string(), json!(true));
            }
        }

        if let Some(u) = tracking_user {
            obj.insert("user".to_string(), json!(u));
        }
    }

    deduplicate_tools(&mut body);

    (body, stream)
}

#[utoipa::path(
    post,
    path = "/responses",
    tag = "chat",
    request_body = serde_json::Value,
    description = "Relay an OpenAI Responses API call to the hosted provider backing the model Bit. Only Bits whose provider declares the `Responses` API surface are accepted.",
    responses(
        (status = 200, description = "LLM response (streaming or JSON)")
    )
)]
#[tracing::instrument(name = "POST /responses", skip_all)]
pub async fn invoke_responses(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<AxumResponse, ApiError> {
    relay_request(
        state,
        user,
        headers,
        payload,
        ModelApiSurface::Responses,
        prepare_upstream_body as PrepareUpstreamBody,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_upstream_body_rewrites_model() {
        let payload = serde_json::json!({
            "model": "bit_123",
            "input": [{"role": "user", "content": "hi"}],
            "stream": false
        });
        let (rewritten, stream) = prepare_upstream_body(
            &payload,
            "gpt-5.2",
            Some("user_123"),
            &HostedProvider::OpenAI,
        );
        assert!(!stream);
        assert_eq!(rewritten.get("model").unwrap().as_str().unwrap(), "gpt-5.2");
        assert_eq!(rewritten.get("user").unwrap().as_str().unwrap(), "user_123");
    }

    #[test]
    fn test_prepare_upstream_body_never_sets_stream_options() {
        let payload = serde_json::json!({
            "model": "bit_123",
            "input": "hi",
            "stream": true
        });
        let (rewritten, stream) =
            prepare_upstream_body(&payload, "gpt-5.2", None, &HostedProvider::OpenAI);
        assert!(stream);
        assert!(rewritten.get("stream_options").is_none());
    }

    #[test]
    fn test_prepare_upstream_body_openrouter_includes_usage() {
        let payload = serde_json::json!({"model": "bit_123", "input": "hi"});
        let (rewritten, _) =
            prepare_upstream_body(&payload, "gpt-5.2", None, &HostedProvider::OpenRouter);
        assert_eq!(
            rewritten
                .get("usage")
                .and_then(|usage| usage.get("include"))
                .and_then(|include| include.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_prepare_upstream_body_deduplicates_flat_tools() {
        let payload = serde_json::json!({
            "model": "bit_123",
            "input": "hi",
            "tools": [
                {"type": "function", "name": "search"},
                {"type": "function", "name": "search"}
            ]
        });
        let (rewritten, _) =
            prepare_upstream_body(&payload, "gpt-5.2", None, &HostedProvider::OpenAI);
        assert_eq!(rewritten.get("tools").unwrap().as_array().unwrap().len(), 1);
    }
}
