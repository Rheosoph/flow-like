use super::{bedrock_auth, hosted_bit_policy, relay::HostedProvider};
use crate::{error::ApiError, state::AppState};
use flow_like::flow_like_model_provider::provider::ModelApiSurface;
use flow_like_secrets::{ExposeSecret, SecretError, SecretRef};
use flow_like_types::reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::time::Duration;

enum HostedAuth {
    ApiKey(String),
    BedrockIam,
}

pub(super) struct HostedConnection {
    pub(super) url: String,
    provider: HostedProvider,
    auth: HostedAuth,
}

impl HostedConnection {
    pub(super) async fn authorize(
        &self,
        state: &AppState,
        bit_id: Option<&str>,
        model_id: &str,
        surface: ModelApiSurface,
        body: &Value,
    ) -> Result<(), ApiError> {
        if surface == ModelApiSurface::SystemOne {
            hosted_bit_policy::authorize_systemone(
                &state.db,
                bit_id.ok_or_else(|| {
                    ApiError::forbidden("System One requires an official catalog Bit")
                })?,
                model_id,
                &self.provider,
                body,
            )
            .await?;
        }
        if matches!(self.auth, HostedAuth::BedrockIam) {
            let bit_id = bit_id.ok_or_else(|| {
                ApiError::forbidden("Bedrock IAM requires an official catalog model")
            })?;
            hosted_bit_policy::authorize_bedrock_iam(&state.db, bit_id, model_id, surface).await?;
            hosted_bit_policy::validate_bedrock_iam_body(body, model_id)?;
        }
        Ok(())
    }

    pub(super) async fn request(
        &self,
        state: &AppState,
        bit_id: Option<&str>,
        model_id: &str,
        surface: ModelApiSurface,
        body: &Value,
        timeout: Duration,
    ) -> Result<RequestBuilder, ApiError> {
        self.authorize(state, bit_id, model_id, surface, body)
            .await?;
        let wire_body = systemone_wire_body(&self.provider, model_id, surface, body)?;
        let client = if surface == ModelApiSurface::SystemOne {
            Client::builder()
                .redirect(flow_like_types::reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| ApiError::internal("Unable to build System One provider client"))?
        } else {
            Client::new()
        };
        let mut request = match &self.auth {
            HostedAuth::ApiKey(api_key) => client
                .post(&self.url)
                .bearer_auth(api_key)
                .json(&wire_body)
                .timeout(timeout),
            HostedAuth::BedrockIam => {
                bedrock_auth::request_builder(state, &self.url, body, timeout).await?
            }
        };
        if self.provider == HostedProvider::OpenRouter {
            request = request
                .header("HTTP-Referer", "https://flow-like.com")
                .header("X-Title", "Flow-Like");
        }
        Ok(request)
    }
}

fn systemone_wire_body(
    provider: &HostedProvider,
    model_id: &str,
    surface: ModelApiSurface,
    body: &Value,
) -> Result<Value, ApiError> {
    if surface != ModelApiSurface::SystemOne {
        return Ok(body.clone());
    }
    if *provider == HostedProvider::OpenRouter {
        let request = super::systemone::validate_payload(body)?
            .with_images_in_state()
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        let mut body = serde_json::to_value(request)?;
        body["model"] = Value::String(model_id.to_owned());
        return Ok(body);
    }
    if *provider != HostedProvider::Cloudflare {
        return Ok(body.clone());
    }
    let mut input = body.clone();
    input
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("Expected System One request object"))?
        .remove("model");
    Ok(serde_json::json!({"model":model_id,"input":input}))
}

async fn optional_secret(state: &AppState, name: &str) -> Result<Option<String>, ApiError> {
    match state.secrets.get_secret_string(&SecretRef::new(name)).await {
        Ok(secret) => Ok(nonempty(Some(secret.expose_secret().to_owned()))),
        Err(SecretError::SecretNotFound(_) | SecretError::NoProvidersConfigured) => Ok(None),
        Err(error) => {
            tracing::warn!(secret_name = name, %error, "Hosted provider configuration unavailable");
            Err(ApiError::service_unavailable(
                "Hosted provider configuration unavailable",
            ))
        }
    }
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) async fn resolve(
    state: &AppState,
    provider: &HostedProvider,
    surface: ModelApiSurface,
) -> Result<HostedConnection, ApiError> {
    let endpoint = optional_secret(state, provider.env_endpoint_key()).await?;
    let api_key = optional_secret(state, provider.env_api_key()).await?;
    #[cfg(feature = "aws")]
    let region = state.aws_client.region().map(|region| region.as_ref());
    #[cfg(not(feature = "aws"))]
    let region = None;
    let connection = configured_connection(provider, surface, endpoint, api_key, region)?;
    #[cfg(not(feature = "aws"))]
    if matches!(connection.auth, HostedAuth::BedrockIam) {
        return Err(ApiError::service_unavailable(
            "Bedrock IAM authentication is unavailable in this server build",
        ));
    }
    Ok(connection)
}

fn configured_connection(
    provider: &HostedProvider,
    surface: ModelApiSurface,
    endpoint: Option<String>,
    api_key: Option<String>,
    aws_region: Option<&str>,
) -> Result<HostedConnection, ApiError> {
    if !super::relay::supports_surface(provider, surface) {
        return Err(ApiError::bad_request(
            "Hosted provider does not support this API surface",
        ));
    }
    let auth = match nonempty(api_key) {
        Some(api_key) => HostedAuth::ApiKey(api_key),
        None if *provider == HostedProvider::Bedrock => HostedAuth::BedrockIam,
        None => {
            return Err(ApiError::internal(format!(
                "{} not configured",
                provider.env_api_key()
            )));
        }
    };
    let endpoint =
        match nonempty(endpoint).or_else(|| provider.default_endpoint().map(String::from)) {
            Some(endpoint) => endpoint,
            None if matches!(auth, HostedAuth::BedrockIam) => {
                bedrock_auth::default_endpoint(aws_region.ok_or_else(|| {
                    ApiError::service_unavailable("Hosted Bedrock region is not configured")
                })?)?
            }
            None => {
                return Err(ApiError::internal(format!(
                    "{} not configured",
                    provider.env_endpoint_key()
                )));
            }
        };
    if *provider == HostedProvider::Cloudflare
        && !(endpoint.len() == 32 && endpoint.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(ApiError::internal("Cloudflare account ID is invalid"));
    }
    let url = provider.endpoint_url(&endpoint, surface);
    if matches!(auth, HostedAuth::BedrockIam) {
        bedrock_auth::validate_target(&url)?;
    }
    Ok(HostedConnection {
        url,
        provider: provider.clone(),
        auth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudflare_wrapper_preserves_questions_without_nested_model_or_chat_fields() {
        let canonical = serde_json::json!({"model":"typesafe/jev","state":"text","questions":{"ok":{"type":"noul","instructions":"Ready?"}}});
        let body = systemone_wire_body(
            &HostedProvider::Cloudflare,
            "typesafe/jev",
            ModelApiSurface::SystemOne,
            &canonical,
        )
        .unwrap();
        assert_eq!(body["model"], "typesafe/jev");
        assert_eq!(body["input"]["questions"], canonical["questions"]);
        assert_eq!(body["input"]["state"], "text");
        assert!(body["input"].get("model").is_none());
        assert_eq!(
            systemone_wire_body(
                &HostedProvider::OpenRouter,
                "typesafe/jev",
                ModelApiSurface::SystemOne,
                &canonical,
            )
            .unwrap(),
            canonical
        );
    }

    #[test]
    fn openrouter_decision_images_use_state_parts_without_changing_other_surfaces() {
        let model = "openai/gpt-6-luna-decisions";
        let image = "data:image/png;base64,AA==";
        let canonical = serde_json::json!({
            "model": model,
            "state": "What color is the image?",
            "images": [image],
            "questions": {"red": {"type": "noul", "instructions": "Is it red?"}}
        });
        let body = systemone_wire_body(
            &HostedProvider::OpenRouter,
            model,
            ModelApiSurface::SystemOne,
            &canonical,
        )
        .unwrap();
        assert_eq!(body["model"], model);
        assert_eq!(body["questions"], canonical["questions"]);
        assert_eq!(
            body["state"],
            serde_json::json!([
                {"type": "text", "text": "What color is the image?"},
                {"type": "image_url", "image_url": {"url": image}}
            ])
        );
        assert!(body.get("images").is_none());
        for surface in [ModelApiSurface::ChatCompletions, ModelApiSurface::Responses] {
            assert_eq!(
                systemone_wire_body(&HostedProvider::OpenRouter, model, surface, &canonical)
                    .unwrap(),
                canonical
            );
        }
        assert_eq!(
            systemone_wire_body(
                &HostedProvider::TypeSafe,
                model,
                ModelApiSurface::SystemOne,
                &canonical,
            )
            .unwrap(),
            canonical
        );
        assert_eq!(
            systemone_wire_body(
                &HostedProvider::Cloudflare,
                model,
                ModelApiSurface::SystemOne,
                &canonical,
            )
            .unwrap()["input"]["images"],
            canonical["images"]
        );
    }

    #[test]
    fn cloudflare_uses_only_a_valid_server_account_id_and_native_surface() {
        let account = "a".repeat(32);
        let connection = configured_connection(
            &HostedProvider::Cloudflare,
            ModelApiSurface::SystemOne,
            Some(account.clone()),
            Some("server-token".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            connection.url,
            format!("https://api.cloudflare.com/client/v4/accounts/{account}/ai/run")
        );
        for endpoint in ["https://attacker.example", "", "abc"] {
            assert!(
                configured_connection(
                    &HostedProvider::Cloudflare,
                    ModelApiSurface::SystemOne,
                    Some(endpoint.into()),
                    Some("server-token".into()),
                    None
                )
                .is_err()
            );
        }
        assert!(
            configured_connection(
                &HostedProvider::Cloudflare,
                ModelApiSurface::ChatCompletions,
                Some(account),
                Some("server-token".into()),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn bedrock_key_takes_precedence_over_iam() {
        let connection = configured_connection(
            &HostedProvider::Bedrock,
            ModelApiSurface::ChatCompletions,
            Some("https://gateway.example/v1".into()),
            Some("configured-key".into()),
            Some("eu-central-1"),
        )
        .unwrap();
        assert!(matches!(connection.auth, HostedAuth::ApiKey(_)));
        assert_eq!(
            connection.url,
            "https://gateway.example/v1/chat/completions"
        );
    }

    #[test]
    fn absent_or_blank_bedrock_key_uses_regional_iam_endpoint() {
        for api_key in [None, Some(" \n".into())] {
            let connection = configured_connection(
                &HostedProvider::Bedrock,
                ModelApiSurface::ChatCompletions,
                None,
                api_key,
                Some("eu-central-1"),
            )
            .unwrap();
            assert!(matches!(connection.auth, HostedAuth::BedrockIam));
            assert_eq!(
                connection.url,
                "https://bedrock-runtime.eu-central-1.amazonaws.com/openai/v1/chat/completions"
            );
        }
    }

    #[test]
    fn configured_iam_endpoint_supplies_its_own_region() {
        let connection = configured_connection(
            &HostedProvider::Bedrock,
            ModelApiSurface::ChatCompletions,
            Some("https://bedrock-mantle.eu-central-1.api.aws/v1".into()),
            None,
            None,
        )
        .unwrap();
        assert!(matches!(connection.auth, HostedAuth::BedrockIam));
    }

    #[test]
    fn unavailable_region_returns_an_error() {
        let result = configured_connection(
            &HostedProvider::Bedrock,
            ModelApiSurface::ChatCompletions,
            None,
            None,
            None,
        );
        assert_eq!(
            result.err().unwrap().status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn iam_never_signs_for_a_custom_gateway() {
        assert!(
            configured_connection(
                &HostedProvider::Bedrock,
                ModelApiSurface::ChatCompletions,
                Some("https://attacker.example/v1".into()),
                None,
                Some("eu-central-1"),
            )
            .is_err()
        );
    }

    #[test]
    fn other_providers_never_fall_back_to_the_aws_role() {
        for provider in [
            HostedProvider::OpenRouter,
            HostedProvider::OpenAI,
            HostedProvider::Azure,
            HostedProvider::Anthropic,
            HostedProvider::Vertex,
        ] {
            assert!(
                configured_connection(
                    &provider,
                    ModelApiSurface::ChatCompletions,
                    Some("https://example.com/v1".into()),
                    None,
                    Some("eu-central-1")
                )
                .is_err()
            );
        }
    }
}
