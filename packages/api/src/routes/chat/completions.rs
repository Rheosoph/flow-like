use super::relay::{HostedProvider, PrepareUpstreamBody, deduplicate_tools, relay_request};
use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State, http::HeaderMap, response::Response as AxumResponse};
use flow_like::flow_like_model_provider::provider::ModelApiSurface;
use flow_like_types::json::json;

/// Anthropic-compatible upstreams reject a conversation that opens on an
/// assistant turn, so a placeholder user turn is inserted ahead of it.
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
            HostedProvider::OpenAI
            | HostedProvider::Anthropic
            | HostedProvider::Azure
            | HostedProvider::Bedrock
            | HostedProvider::Vertex => {
                if stream {
                    enable_stream_usage_options(obj);
                }
            }
        }

        if let Some(u) = tracking_user {
            obj.insert("user".to_string(), json!(u));
        }
    }

    deduplicate_tools(&mut body);
    ensure_user_first_message(&mut body);

    (body, stream)
}

#[utoipa::path(
    post,
    path = "/chat/completions",
    tag = "chat",
    request_body = serde_json::Value,
    description = "Relay an OpenAI-compatible chat completion to the hosted provider backing the model Bit.",
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
    relay_request(
        state,
        user,
        headers,
        payload,
        ModelApiSurface::ChatCompletions,
        prepare_upstream_body as PrepareUpstreamBody,
    )
    .await
}

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
    fn test_prepare_upstream_body_anthropic_uses_openai_compatibility() {
        let payload = serde_json::json!({"model": "bit_123", "messages": [], "stream": true});
        let (rewritten, stream) =
            prepare_upstream_body(&payload, "claude-3-opus", None, &HostedProvider::Anthropic);
        assert!(stream);
        assert!(rewritten.get("max_tokens").is_none());
        assert_eq!(
            rewritten
                .get("stream_options")
                .and_then(|options| options.get("include_usage"))
                .and_then(|include| include.as_bool()),
            Some(true)
        );
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
