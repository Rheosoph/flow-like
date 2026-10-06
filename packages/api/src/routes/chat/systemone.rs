//! Hosted typed decisions with the same catalog authority and metering as chat.

use super::relay::{HostedProvider, PrepareUpstreamBody, relay_instance_request, relay_request};
use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State, http::HeaderMap, response::Response};
use flow_like::flow_like_model_provider::{provider::ModelApiSurface, systemone::SystemOneRequest};
use serde_json::Value;

pub(super) fn validate_payload(payload: &Value) -> Result<SystemOneRequest, ApiError> {
    let mut request = payload.clone();
    let object = request
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("Expected a System One request object"))?;
    let model = object
        .remove("model")
        .ok_or_else(|| ApiError::bad_request("Missing model Bit ID"))?;
    if model.as_str().is_none_or(|model| model.trim().is_empty()) {
        return Err(ApiError::bad_request("Model Bit ID must not be empty"));
    }
    let request: SystemOneRequest = serde_json::from_value(request)
        .map_err(|error| ApiError::bad_request(format!("Invalid System One request: {error}")))?;
    request
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(request)
}

pub(super) fn normalize_response(
    bytes: &[u8],
    payload: &Value,
    provider: &str,
) -> Result<flow_like_types::Bytes, ApiError> {
    use flow_like::flow_like_model_provider::systemone::SystemOneResponse;
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|_| ApiError::internal("System One provider returned invalid JSON"))?;
    if provider == "cloudflare" {
        if value.get("success").and_then(Value::as_bool) == Some(false) {
            return Err(ApiError::internal("Cloudflare System One inference failed"));
        }
        if let Some(result) = value.get_mut("result") {
            value = result.take();
        }
    }
    let response: SystemOneResponse = serde_json::from_value(value)
        .map_err(|_| ApiError::internal("System One provider returned invalid typed answers"))?;
    response
        .validate_for(&validate_payload(payload)?)
        .map_err(|_| {
            ApiError::internal("System One provider returned mismatched or invalid decisions")
        })?;
    Ok(flow_like_types::Bytes::from(serde_json::to_vec(&response)?))
}

fn prepare_upstream_body(
    payload: &Value,
    model: &str,
    _: Option<&str>,
    _: &HostedProvider,
) -> (Value, bool) {
    let mut body = payload.clone();
    body["model"] = Value::String(model.to_owned());
    (body, false)
}

#[utoipa::path(post, path = "/systemone", tag = "models", request_body = serde_json::Value,
    description = "Evaluate typed questions with a hosted System One model Bit.",
    responses((status = 200, description = "Typed answers, probabilities, and usage")))]
#[tracing::instrument(name = "POST /systemone", skip_all)]
pub async fn invoke_systemone(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Response, ApiError> {
    validate_payload(&payload)?;
    relay_request(
        state,
        user,
        headers,
        payload,
        ModelApiSurface::SystemOne,
        prepare_upstream_body as PrepareUpstreamBody,
    )
    .await
}

pub async fn invoke_instance_systemone(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Response, ApiError> {
    validate_payload(&payload)?;
    relay_instance_request(
        state,
        headers,
        payload,
        ModelApiSurface::SystemOne,
        "/instances/systemone",
        prepare_upstream_body as PrepareUpstreamBody,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> Value {
        json!({"model":"bit-id", "state":{"text":"charged twice"}, "questions":{"refund":{"type":"noul","instructions":"Was the customer charged twice?"}}})
    }
    #[test]
    fn cloudflare_wrapped_answers_and_usage_are_normalized_and_checked() {
        let native = json!({"model":"jev", "answers":{"refund":{"type":"noul","noul":0.9}},
            "usage":{"input_tokens":50,"output_tokens":12}});
        for value in [native.clone(), json!({"success":true,"result":native})] {
            let bytes = normalize_response(
                &serde_json::to_vec(&value).unwrap(),
                &request(),
                "cloudflare",
            )
            .unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(result["answers"]["refund"]["noul"], 0.9);
            assert_eq!(result["usage"]["output_tokens"], 12);
            assert!(result.get("result").is_none());
        }
        for invalid in [
            json!({"success":false,"result":native}),
            json!({"model":"jev","answers":{}}),
            json!({"model":"jev","answers":{"refund":{"type":"noul","noul":2}}}),
        ] {
            assert!(
                normalize_response(
                    &serde_json::to_vec(&invalid).unwrap(),
                    &request(),
                    "cloudflare"
                )
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_chat_and_transport_extensions_but_preserves_nested_data() {
        for field in [
            "stream",
            "endpoint",
            "headers",
            "provider",
            "max_tokens",
            "messages",
        ] {
            let mut body = request();
            body[field] = json!(true);
            assert!(validate_payload(&body).is_err(), "{field}");
        }
        let mut body = request();
        body["state"]["endpoint"] = json!("ordinary domain data");
        assert!(validate_payload(&body).is_ok());
    }
    #[test]
    fn relays_only_the_decision_contract_and_replaces_model() {
        let (body, stream) = prepare_upstream_body(
            &request(),
            "typesafe/jev-1.13",
            Some("user"),
            &HostedProvider::OpenRouter,
        );
        assert!(!stream);
        assert_eq!(body["model"], "typesafe/jev-1.13");
        assert!(
            body.get("user").is_none()
                && body.get("usage").is_none()
                && body.get("max_tokens").is_none()
        );
        assert_eq!(body["questions"], request()["questions"]);
    }
}
