//! Gemini models on Vertex AI, called over Vertex AI's own `generateContent` REST API with a
//! service account key, a static access token or application default credentials.

mod client;
mod media;
mod request;
mod response;
mod wire;

use std::{any::Any, future::Future};

use super::ModelLogic;
use crate::provider::random_provider;
use crate::{
    egress::ProviderEgress,
    history::{History, HistoryThinking},
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration, VertexConfig},
};
use anyhow::{Result, anyhow, ensure};
use async_trait::async_trait;
use client::{GLOBAL_LOCATION, Target, VertexClient};
use flow_like_types_contracts::Cacheable;
use google_cloud_auth::credentials::{
    self as google_credentials, CacheableResource, Credentials, CredentialsProvider, EntityTag,
    impersonated, service_account,
};
use google_cloud_auth::errors::CredentialsError;
use http::{Extensions, HeaderMap, HeaderValue, header::AUTHORIZATION};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct StaticAccessTokenCredentials {
    token: String,
}

impl CredentialsProvider for StaticAccessTokenCredentials {
    fn headers(
        &self,
        _extensions: Extensions,
    ) -> impl Future<Output = std::result::Result<CacheableResource<HeaderMap>, CredentialsError>> + Send
    {
        let token = self.token.clone();
        async move {
            let mut headers = HeaderMap::new();
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| {
                CredentialsError::from_msg(false, format!("Invalid Vertex access token: {e}"))
            })?;
            headers.insert(AUTHORIZATION, value);
            Ok(CacheableResource::New {
                entity_tag: EntityTag::new(),
                data: headers,
            })
        }
    }

    async fn universe_domain(&self) -> Option<String> {
        Some(DEFAULT_UNIVERSE_DOMAIN.to_string())
    }
}

pub struct VertexModel {
    client: VertexClient,
    default_model: Option<String>,
}

impl VertexModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let vertex_config = if config.vertex_config.is_empty() {
            VertexConfig::default()
        } else {
            random_provider(&config.vertex_config)?
        };

        let client = build_client(
            vertex_config.project_id.as_deref(),
            vertex_config.location.as_deref(),
            vertex_config.service_account_json.as_deref(),
            vertex_config.access_token.as_deref(),
            ProviderEgress::Open,
        )?;

        Ok(VertexModel {
            client,
            default_model: provider.model_id.clone(),
        })
    }

    /// Under a guarded `egress` the Vertex AI calls go through the egress resolver, and the
    /// target is pinned to Google Cloud as well: the project and location must be Google
    /// identifiers and a service account key must belong to the default universe.
    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let project_id =
            string_param(&params, "project_id").or_else(|| string_param(&params, "project"));
        let location =
            string_param(&params, "location").or_else(|| string_param(&params, "region"));
        let service_account_json = string_param(&params, "service_account_json")
            .or_else(|| string_param(&params, "service_account_key"));
        let access_token = string_param(&params, "access_token");
        let model_id = string_param(&params, "model_id").or_else(|| provider.model_id.clone());

        let project_id = project_id.or_else(|| {
            service_account_json
                .as_deref()
                .and_then(service_account_project_id)
        });

        if egress != ProviderEgress::Open {
            validate_target(
                project_id.as_deref(),
                location.as_deref(),
                service_account_json.as_deref(),
            )?;
        }

        let client = build_client(
            project_id.as_deref(),
            location.as_deref(),
            service_account_json.as_deref(),
            access_token.as_deref(),
            egress,
        )?
        .with_request_params(provider)?;

        Ok(VertexModel {
            client,
            default_model: model_id,
        })
    }
}

impl Cacheable for VertexModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for VertexModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    /// The History's thinking level as Vertex AI's `thinkingConfig`. Its OpenAI-style fields
    /// have no place in a Vertex AI request.
    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let thinking = history.thinking?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        Some(json!({ "thinkingConfig": thinking_config(thinking, model) }))
    }

    fn transform_history(&self, history: &mut History) {
        media::note_unreadable_attachments(history);
    }
}

/// `MINIMAL` exists only on the Flash and Flash-Lite Gemini 3 models; 3 Pro and 3.1 Pro cannot
/// turn thinking off at all.
fn accepts_minimal_thinking(model: &str) -> bool {
    model.contains("flash-lite")
        || ["gemini-3-flash", "gemini-3.5-flash", "gemini-3.6-flash"]
            .iter()
            .any(|family| model.contains(family))
}

/// Gemini 3 takes a thinking level, earlier models a token budget. Gemini 2.5 Pro cannot turn
/// thinking off; its smallest budget is 128.
fn thinking_config(thinking: HistoryThinking, model: &str) -> Value {
    let model = model.to_ascii_lowercase();
    let include_thoughts = thinking != HistoryThinking::Off;
    if model.contains("gemini-3") {
        let level = match thinking {
            _ if model.contains("pro-image") => "HIGH",
            HistoryThinking::Off if accepts_minimal_thinking(&model) => "MINIMAL",
            HistoryThinking::Off | HistoryThinking::Low => "LOW",
            HistoryThinking::Mid => "MEDIUM",
            HistoryThinking::High => "HIGH",
        };
        json!({ "includeThoughts": include_thoughts, "thinkingLevel": level })
    } else {
        let budget = match thinking {
            HistoryThinking::Off if model.contains("2.5-pro") => 128,
            HistoryThinking::Off => 0,
            HistoryThinking::Low => 1024,
            HistoryThinking::Mid => 2048,
            HistoryThinking::High => 4096,
        };
        json!({ "includeThoughts": include_thoughts, "thinkingBudget": budget })
    }
}

fn string_param(
    params: &std::collections::HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn service_account_project_id(service_account_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(service_account_json)
        .ok()
        .and_then(|value| {
            value
                .get("project_id")
                .and_then(|project_id| project_id.as_str())
                .map(str::trim)
                .filter(|project_id| !project_id.is_empty())
                .map(ToOwned::to_owned)
        })
}

const DEFAULT_UNIVERSE_DOMAIN: &str = "googleapis.com";

/// The project and location become segments of the request path, and a service account
/// key from another universe points the client at that universe's hosts. Refusing both
/// keeps a guarded caller on `aiplatform.googleapis.com`.
fn validate_target(
    project_id: Option<&str>,
    location: Option<&str>,
    service_account_json: Option<&str>,
) -> Result<()> {
    if let Some(project) = project_id
        && !is_resource_segment(project, &['.', ':'])
    {
        anyhow::bail!("Vertex project '{project}' is not a Google Cloud project id");
    }
    if let Some(location) = location
        && !is_resource_segment(location, &[])
    {
        anyhow::bail!(
            "Vertex location '{location}' is not a Google Cloud location such as 'us-central1'"
        );
    }
    let universe = service_account_json
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|key| {
            key.get("universe_domain")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        });
    if let Some(universe) = universe.filter(|universe| universe != DEFAULT_UNIVERSE_DOMAIN) {
        anyhow::bail!(
            "Vertex service accounts must belong to the {DEFAULT_UNIVERSE_DOMAIN} universe, not '{universe}'"
        );
    }
    Ok(())
}

fn is_resource_segment(value: &str, extra: &[char]) -> bool {
    value.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && !value.contains("..")
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || extra.contains(&c))
}

/// The location and universe are part of the API host name, so even an unguarded client keeps
/// them to the characters of a DNS name.
fn is_host_name_part(value: &str) -> bool {
    value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn credentials_from_params(
    service_account_json: Option<&str>,
    access_token: Option<&str>,
) -> Result<Option<Credentials>> {
    if let Some(service_account_json) = service_account_json {
        let key = serde_json::from_str::<serde_json::Value>(service_account_json)
            .map_err(|e| anyhow!("Invalid Vertex service account JSON: {e}"))?;
        let credentials = service_account::Builder::new(key)
            .build()
            .map_err(|e| anyhow!("Failed to build Vertex service account credentials: {e}"))?;
        return Ok(Some(credentials));
    }

    if let Some(access_token) = access_token {
        return Ok(Some(Credentials::from(StaticAccessTokenCredentials {
            token: access_token.to_string(),
        })));
    }

    Ok(None)
}

/// Application default credentials, impersonating `GOOGLE_CLOUD_SERVICE_ACCOUNT` when it is
/// set, as rig-vertexai resolved them.
fn application_default_credentials() -> Result<Credentials> {
    let source = google_credentials::Builder::default()
        .build()
        .map_err(|e| anyhow!("Failed to load Google application default credentials: {e}"))?;
    match env_value("GOOGLE_CLOUD_SERVICE_ACCOUNT") {
        Some(account) => impersonated::Builder::from_source_credentials(source)
            .with_target_principal(account)
            .build()
            .map_err(|e| anyhow!("Failed to impersonate the Vertex service account: {e}")),
        None => Ok(source),
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn universe_domain(service_account_json: Option<&str>) -> String {
    service_account_json
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
        .and_then(|key| {
            key.get("universe_domain")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|universe| !universe.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_UNIVERSE_DOMAIN.to_string())
}

/// The project and location fall back to `GOOGLE_CLOUD_PROJECT` and `GOOGLE_CLOUD_LOCATION`,
/// then to the global location, as with rig-vertexai.
fn build_client(
    project_id: Option<&str>,
    location: Option<&str>,
    service_account_json: Option<&str>,
    access_token: Option<&str>,
    egress: ProviderEgress,
) -> Result<VertexClient> {
    let service_account_json = non_empty(service_account_json);
    let project = non_empty(project_id)
        .map(str::to_owned)
        .or_else(|| env_value("GOOGLE_CLOUD_PROJECT"))
        .ok_or_else(|| {
            anyhow!(
                "Vertex AI needs a Google Cloud project: set the project id, use a service \
                 account key that names one, or set GOOGLE_CLOUD_PROJECT"
            )
        })?;
    let location = non_empty(location)
        .map(str::to_owned)
        .or_else(|| env_value("GOOGLE_CLOUD_LOCATION"))
        .unwrap_or_else(|| GLOBAL_LOCATION.to_string());
    ensure!(
        is_host_name_part(&location),
        "Vertex location '{location}' is not a Google Cloud location such as 'us-central1'"
    );
    let universe = universe_domain(service_account_json);
    ensure!(
        is_host_name_part(&universe),
        "Vertex service account universe '{universe}' is not a domain"
    );
    let credentials = match credentials_from_params(service_account_json, non_empty(access_token))?
    {
        Some(credentials) => credentials,
        None => application_default_credentials()?,
    };
    VertexClient::new(
        Target {
            project,
            location,
            universe,
        },
        credentials,
        egress,
    )
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::HistoryMessage;
    use serde_json::json;
    use std::collections::HashMap;

    fn provider(location: &str) -> ModelProvider {
        ModelProvider {
            provider_name: "custom:vertex".to_string(),
            model_id: Some("gemini-2.5-flash".to_string()),
            version: None,
            api_surface: None,
            params: Some(HashMap::from([
                ("project_id".to_string(), json!("my-project")),
                ("location".to_string(), json!(location)),
                ("access_token".to_string(), json!("ya29.token")),
            ])),
        }
    }

    #[test]
    fn guarded_targets_stay_on_google_cloud() {
        for (project, location) in [
            ("my-project", "us-central1"),
            ("example.com:my-project", "global"),
            ("123456789012", "europe-west4"),
        ] {
            assert!(
                validate_target(Some(project), Some(location), None).is_ok(),
                "{project} {location}"
            );
        }
        for project in [
            "",
            "My-Project",
            "-leading",
            "../admin",
            "a..b",
            "proj/locations/x",
            "proj?alt=media",
            "proj#fragment",
            "proj name",
        ] {
            assert!(
                validate_target(Some(project), None, None).is_err(),
                "{project}"
            );
        }
        for location in [
            "",
            "US-CENTRAL1",
            "us-central1.attacker.example",
            "us/central1",
            "..",
            "global?x=1",
            "eu:1",
        ] {
            assert!(
                validate_target(None, Some(location), None).is_err(),
                "{location}"
            );
        }
        let key = |universe: &str| json!({"type": "service_account", "universe_domain": universe});
        assert!(validate_target(None, None, Some(&key("googleapis.com").to_string())).is_ok());
        assert!(validate_target(None, None, Some(&json!({}).to_string())).is_ok());
        assert!(validate_target(None, None, Some(&key("attacker.example").to_string())).is_err());
    }

    #[tokio::test]
    async fn only_guarded_constructors_validate_the_target() {
        let hostile = provider("us-central1.attacker.example");
        assert!(
            VertexModel::from_provider(&hostile, ProviderEgress::Public)
                .await
                .is_err()
        );
        assert!(
            VertexModel::from_provider(&hostile, ProviderEgress::Open)
                .await
                .is_ok()
        );
        assert!(
            VertexModel::from_provider(&provider("us-central1"), ProviderEgress::Public)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn locations_pick_the_api_host_and_cannot_leave_it() {
        for (location, host) in [
            ("global", "https://aiplatform.googleapis.com"),
            (
                "us-central1",
                "https://us-central1-aiplatform.googleapis.com",
            ),
            (
                "us-central1.attacker.example",
                "https://us-central1.attacker.example-aiplatform.googleapis.com",
            ),
        ] {
            let model = VertexModel::from_provider(&provider(location), ProviderEgress::Open)
                .await
                .unwrap();
            assert_eq!(model.client.base_url(), host);
        }
        for location in [
            "evil.example/x#",
            "evil.example?",
            "user@evil.example",
            "..",
        ] {
            assert!(
                VertexModel::from_provider(&provider(location), ProviderEgress::Open)
                    .await
                    .is_err(),
                "{location}"
            );
        }
        let mut other_universe = provider("global");
        other_universe.params.as_mut().unwrap().insert(
            "service_account_json".into(),
            json!(json!({"universe_domain": "evil.example/#"}).to_string()),
        );
        assert!(
            VertexModel::from_provider(&other_universe, ProviderEgress::Open)
                .await
                .is_err()
        );
    }

    #[test]
    fn thinking_levels_map_to_each_model_family() {
        let config = |thinking, model| thinking_config(thinking, model);
        assert_eq!(
            config(HistoryThinking::Off, "gemini-2.5-flash"),
            json!({"includeThoughts": false, "thinkingBudget": 0})
        );
        assert_eq!(
            config(HistoryThinking::Off, "gemini-2.5-pro"),
            json!({"includeThoughts": false, "thinkingBudget": 128})
        );
        assert_eq!(
            config(HistoryThinking::High, "gemini-2.5-flash"),
            json!({"includeThoughts": true, "thinkingBudget": 4096})
        );
        assert_eq!(
            config(HistoryThinking::Off, "gemini-3-flash-preview"),
            json!({"includeThoughts": false, "thinkingLevel": "MINIMAL"})
        );
        assert_eq!(
            config(HistoryThinking::Off, "gemini-3.1-pro-preview"),
            json!({"includeThoughts": false, "thinkingLevel": "LOW"})
        );
        assert_eq!(
            config(HistoryThinking::Mid, "gemini-3-pro-preview"),
            json!({"includeThoughts": true, "thinkingLevel": "MEDIUM"})
        );
        assert_eq!(
            config(HistoryThinking::Low, "gemini-3-pro-image-preview")["thinkingLevel"],
            "HIGH"
        );
    }

    #[tokio::test]
    async fn only_a_thinking_level_becomes_a_request_parameter() {
        let model = VertexModel::from_provider(&provider("global"), ProviderEgress::Open)
            .await
            .unwrap();
        let mut history = History::new(
            "ignored".into(),
            vec![HistoryMessage::from_string(
                crate::history::Role::User,
                "Hi",
            )],
        );
        history.top_p = Some(0.5);
        assert_eq!(model.additional_params(&Some(history.clone())), None);
        history.thinking = Some(HistoryThinking::Low);
        assert_eq!(
            model.additional_params(&Some(history)),
            Some(json!({"thinkingConfig": {"includeThoughts": true, "thinkingBudget": 1024}}))
        );
    }

    #[tokio::test]
    async fn invoke_streams_with_the_thinking_level_and_notes_unreadable_attachments() {
        use crate::history::{Content, ContentType, MessageContent, Role};
        use crate::llm::LLMCallback;
        use crate::llm::test_support::{serve_once, sse_response};
        use std::sync::{Arc, Mutex};

        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        let (endpoint, server) = serve_once(sse_response(&[
            json!({"candidates": [{"content": {"role": "model", "parts": [
                {"text": "Reading.", "thought": true},
            ]}}]}),
            json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Hello"}]},
                "finishReason": "STOP"}],
                "usageMetadata": {"promptTokenCount": 8, "candidatesTokenCount": 1,
                    "totalTokenCount": 12, "thoughtsTokenCount": 3}}),
        ]))
        .await;
        let model = VertexModel {
            client: VertexClient::for_tests(
                &endpoint,
                Credentials::from(StaticAccessTokenCredentials {
                    token: "ya29.test".into(),
                }),
                Arc::new(media::fake::FakeNetwork::default()),
            ),
            default_model: Some("gemini-2.5-flash".into()),
        };
        let mut history = History::new(
            "gemini-2.5-flash".into(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(vec![
                    Content::Text {
                        content_type: ContentType::Text,
                        text: "Summarise this.".into(),
                    },
                    Content::Document {
                        content_type: ContentType::DocumentUrl,
                        document_url: format!("data:{docx};base64,UEsDBA=="),
                        media_type: Some(docx.into()),
                        additional_params: None,
                    },
                ]),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );
        history.thinking = Some(HistoryThinking::Low);
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink = chunks.clone();
        let callback: LLMCallback = Arc::new(move |chunk| {
            sink.lock().unwrap().push(chunk);
            Box::pin(async { Ok(()) })
        });

        let response = model.invoke(&history, Some(callback)).await.unwrap();
        assert_eq!(response.content().as_deref(), Some("Hello"));
        assert_eq!(
            response
                .last_message()
                .and_then(|message| message.reasoning.as_deref()),
            Some("Reading.")
        );
        assert_eq!(response.usage.prompt_tokens, 8);
        assert_eq!(chunks.lock().unwrap().len(), 3);

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            body["generationConfig"],
            json!({"candidateCount": 1,
                "thinkingConfig": {"includeThoughts": true, "thinkingBudget": 1024}})
        );
        assert_eq!(
            body["contents"],
            json!([{"role": "user", "parts": [
                {"text": "Summarise this."},
                {"text": format!("[attachment omitted: unsupported file type {docx}]")},
            ]}])
        );
    }
}
