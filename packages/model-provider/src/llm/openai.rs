use std::{any::Any, sync::Arc};

use super::{
    ModelLogic, UsageReportingMode, body_params, drop_body_param, drop_temperature,
    extract_headers, merge_additional_params, output_budget_as_body_param,
};
use crate::authorization::AuthorizedHttpClient;
use crate::provider::random_provider;
use crate::{
    history::{History, HistoryThinking},
    llm::ModelConstructor,
    provider::{ModelApiSurface, ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use flow_like_types_contracts::authorization::{RequestAuthorizer, ResourceAudience};
use rig::completion::CompletionRequest;
use serde_json::{Value, json};

#[derive(Clone)]
enum OpenAIClientType {
    OpenAI(rig::providers::openai::Client<AuthorizedHttpClient>),
    OpenAIChatCompletions(rig::providers::openai::CompletionsClient<AuthorizedHttpClient>),
    Azure(rig::providers::azure::Client),
}

/// Reasoning support by model family, from the model id alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAIReasoning {
    /// gpt-4o, gpt-4.1, gpt-5-chat, o1-mini: reasoning control returns 400.
    Unsupported,
    OSeries,
    Gpt5 {
        minor: u32,
    },
    Gpt6,
    GptOss,
    /// Hosted Bit ids and other names that don't reveal the upstream model.
    Unknown,
}

impl OpenAIReasoning {
    fn of(model: &str) -> Self {
        let model = model.to_ascii_lowercase();
        let name = model.rsplit('/').next().unwrap_or(&model);
        let name = name.strip_prefix("openai.").unwrap_or(name);
        if name.contains("gpt-oss") {
            return Self::GptOss;
        }
        if name.contains("gpt-6") {
            return Self::Gpt6;
        }
        if let Some(rest) = name.split_once("gpt-5").map(|(_, rest)| rest) {
            if rest.contains("-chat") {
                return Self::Unsupported;
            }
            let minor = rest
                .strip_prefix('.')
                .map(|rest| {
                    rest.chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                })
                .and_then(|digits| digits.parse().ok())
                .unwrap_or(0);
            return Self::Gpt5 { minor };
        }
        if name.starts_with("o1-mini") {
            return Self::Unsupported;
        }
        let mut chars = name.chars();
        if chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit()) {
            return Self::OSeries;
        }
        if ["gpt-4", "gpt-3.5", "chatgpt"]
            .iter()
            .any(|family| name.contains(family))
        {
            return Self::Unsupported;
        }
        Self::Unknown
    }

    /// Effort names differ per family: `minimal` exists only on gpt-5, `none` from gpt-5.1 on.
    fn effort(self, thinking: HistoryThinking) -> Option<&'static str> {
        match (self, thinking) {
            (Self::Unsupported, _) => None,
            (_, HistoryThinking::Low) => Some("low"),
            (_, HistoryThinking::Mid) => Some("medium"),
            (_, HistoryThinking::High) => Some("high"),
            (Self::Gpt5 { minor: 0 }, HistoryThinking::Off) => Some("minimal"),
            (Self::Gpt5 { .. } | Self::Unknown, HistoryThinking::Off) => Some("none"),
            (Self::OSeries | Self::Gpt6 | Self::GptOss, HistoryThinking::Off) => Some("low"),
        }
    }

    /// o-series and GPT-5+ reject `temperature`/`top_p`/penalties unless gpt-5.1+ runs with
    /// reasoning effort `none`.
    fn rejects_sampling(self, effort: Option<&str>) -> bool {
        match self {
            Self::OSeries | Self::Gpt6 => true,
            Self::Gpt5 { minor } => !(minor >= 1 && effort == Some("none")),
            Self::Unsupported | Self::GptOss | Self::Unknown => false,
        }
    }

    /// Chat Completions reasoning models accept only `max_completion_tokens`.
    fn needs_max_completion_tokens(self) -> bool {
        matches!(self, Self::OSeries | Self::Gpt5 { .. } | Self::Gpt6)
    }
}

fn drop_rejected_sampling(model: &str, request: &mut CompletionRequest) {
    let effort = request.additional_params.as_ref().and_then(|params| {
        params
            .get("reasoning_effort")
            .or_else(|| params.pointer("/reasoning/effort"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    if !OpenAIReasoning::of(model).rejects_sampling(effort.as_deref()) {
        return;
    }
    drop_temperature(request, model, "rejected by reasoning models");
    for key in ["top_p", "presence_penalty", "frequency_penalty"] {
        drop_body_param(request, key, model, "rejected by reasoning models");
    }
}

fn chat_completions_constraints(model: &str, mut request: CompletionRequest) -> CompletionRequest {
    if OpenAIReasoning::of(model).needs_max_completion_tokens()
        && let Some(max_tokens) = request.max_tokens.take()
    {
        body_params(&mut request).insert("max_completion_tokens".into(), json!(max_tokens));
    }
    drop_rejected_sampling(model, &mut request);
    request
}

/// Rig's Azure request omits `max_tokens`, so the budget always travels as a body field.
fn azure_constraints(model: &str, request: CompletionRequest) -> CompletionRequest {
    let key = if OpenAIReasoning::of(model).needs_max_completion_tokens() {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    let mut request = output_budget_as_body_param(request, key);
    drop_rejected_sampling(model, &mut request);
    request
}

fn responses_constraints(model: &str, mut request: CompletionRequest) -> CompletionRequest {
    drop_rejected_sampling(model, &mut request);
    request
}

pub struct OpenAIModel {
    client: OpenAIClientType,
    default_model: Option<String>,
}

impl OpenAIModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let openai_config = random_provider(&config.openai_config)?;
        let api_key = openai_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let client = if provider.provider_name == "azure" {
            let endpoint = openai_config.endpoint.clone().unwrap_or_default();
            let endpoint = if endpoint.ends_with('/') {
                endpoint.to_string()
            } else {
                format!("{}/", endpoint)
            };

            let auth = rig::providers::azure::AzureOpenAIAuth::ApiKey(api_key.clone());
            let mut builder = rig::providers::azure::Client::builder()
                .api_key(auth)
                .azure_endpoint(endpoint);
            if let Some(version) = provider.version.as_deref() {
                builder = builder.api_version(version);
            }

            OpenAIClientType::Azure(builder.build()?)
        } else {
            let mut builder = rig::providers::openai::Client::builder()
                .api_key(&api_key)
                .http_client(AuthorizedHttpClient::default());
            if let Some(endpoint) = openai_config.endpoint.as_deref() {
                builder = builder.base_url(endpoint);
            }

            OpenAIClientType::OpenAI(builder.build()?)
        };

        Ok(OpenAIModel {
            client,
            default_model: model_id,
        })
    }

    #[allow(clippy::cognitive_complexity)]
    pub async fn from_provider(provider: &ModelProvider) -> anyhow::Result<Self> {
        Self::from_provider_with_authorizer(provider, None).await
    }

    #[allow(clippy::cognitive_complexity)]
    pub async fn from_provider_with_authorizer(
        provider: &ModelProvider,
        authorizer: Option<Arc<dyn RequestAuthorizer>>,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let is_azure = params.get("is_azure").cloned();
        let endpoint = params.get("endpoint").cloned();
        let custom_headers = extract_headers(&params);

        let is_azure = match is_azure {
            Some(val) => val.as_bool().unwrap_or(false),
            None => false,
        };

        if is_azure && authorizer.is_some() {
            anyhow::bail!("Renewable hosted authorization requires the Flow-Like proxy endpoint");
        }

        if is_azure && endpoint.is_none() {
            return Err(anyhow::anyhow!("Azure OpenAI requires an endpoint"));
        }

        if is_azure && model_id.is_none() {
            return Err(anyhow::anyhow!(
                "Azure OpenAI requires a model_id (deployment name)"
            ));
        }

        let client = if is_azure {
            let endpoint = endpoint.unwrap_or_default();
            let endpoint = endpoint.as_str().unwrap_or_default();
            let endpoint = if endpoint.ends_with('/') {
                endpoint.to_string()
            } else {
                format!("{}/", endpoint)
            };

            let auth = rig::providers::azure::AzureOpenAIAuth::ApiKey(api_key.to_string());
            let mut builder = rig::providers::azure::Client::builder()
                .api_key(auth)
                .azure_endpoint(endpoint);
            if let Some(version_str) = params.get("version").and_then(|v| v.as_str()) {
                builder = builder.api_version(version_str);
            }
            if !custom_headers.is_empty() {
                builder = builder.http_headers(custom_headers);
            }
            OpenAIClientType::Azure(builder.build()?)
        } else {
            let http_client = match authorizer {
                Some(authorizer) => AuthorizedHttpClient::new(
                    authorizer,
                    ResourceAudience::HostedModels,
                    endpoint
                        .as_ref()
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!("Hosted authorization requires an explicit endpoint")
                        })?,
                )?,
                None => AuthorizedHttpClient::default(),
            };
            let mut builder = rig::providers::openai::Client::builder()
                .api_key(api_key)
                .http_client(http_client);
            if let Some(endpoint) = endpoint.as_ref().and_then(|v| v.as_str()) {
                builder = builder.base_url(endpoint);
            }
            if !custom_headers.is_empty() {
                builder = builder.http_headers(custom_headers);
            }
            OpenAIClientType::OpenAI(builder.build()?)
        };

        Ok(OpenAIModel {
            client,
            default_model: model_id,
        })
    }

    /// Build an OpenAI client that uses the Chat Completions API.
    ///
    /// Rig's default OpenAI client uses the Responses API. Flow-Like's hosted
    /// model proxy exposes an OpenAI-compatible `/chat/completions` endpoint,
    /// so hosted OpenAI Bits must opt into Rig's completions client explicitly.
    /// Direct OpenAI models keep using [`Self::from_provider`] and the Responses
    /// API.
    pub async fn from_provider_chat_completions(provider: &ModelProvider) -> anyhow::Result<Self> {
        Ok(Self::from_provider(provider)
            .await?
            .with_api_surface(ModelApiSurface::ChatCompletions))
    }

    /// Build the client for an explicit API surface.
    ///
    /// [`ModelApiSurface::Responses`] keeps Rig's default OpenAI client, which
    /// posts to `{base_url}/responses`; [`ModelApiSurface::ChatCompletions`]
    /// swaps in the completions client, which posts to
    /// `{base_url}/chat/completions`.
    pub async fn from_provider_with_surface(
        provider: &ModelProvider,
        surface: ModelApiSurface,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_provider(provider)
            .await?
            .with_api_surface(surface))
    }

    pub async fn from_provider_with_surface_and_authorizer(
        provider: &ModelProvider,
        surface: ModelApiSurface,
        authorizer: Option<Arc<dyn RequestAuthorizer>>,
    ) -> anyhow::Result<Self> {
        Ok(Self::from_provider_with_authorizer(provider, authorizer)
            .await?
            .with_api_surface(surface))
    }

    /// Re-target an already built client at `surface`.
    ///
    /// Azure clients are left untouched — Rig models Azure as a single
    /// deployment client that has no Responses counterpart.
    pub fn with_api_surface(self, surface: ModelApiSurface) -> Self {
        let Self {
            client,
            default_model,
        } = self;
        let client = match (client, surface) {
            (OpenAIClientType::OpenAI(client), ModelApiSurface::ChatCompletions) => {
                OpenAIClientType::OpenAIChatCompletions(client.completions_api())
            }
            (OpenAIClientType::OpenAIChatCompletions(client), ModelApiSurface::Responses) => {
                OpenAIClientType::OpenAI(client.responses_api())
            }
            (other, _) => other,
        };

        Self {
            client,
            default_model,
        }
    }
}

impl Cacheable for OpenAIModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for OpenAIModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(match self.client.clone() {
            OpenAIClientType::OpenAI(client) => {
                ModelConstructor::with_request_fixup(client, responses_constraints)
            }
            OpenAIClientType::OpenAIChatCompletions(client) => {
                ModelConstructor::with_request_fixup(client, chat_completions_constraints)
            }
            OpenAIClientType::Azure(client) => {
                ModelConstructor::with_request_fixup(client, azure_constraints)
            }
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        // The Responses API rejects `stream_options` and always emits usage on
        // the terminal `response.completed` event, so only the Chat Completions
        // and Azure clients need the opt-in.
        match self.client {
            OpenAIClientType::OpenAI(_) => UsageReportingMode::None,
            OpenAIClientType::OpenAIChatCompletions(_) | OpenAIClientType::Azure(_) => {
                UsageReportingMode::OpenAIStreamOptions
            }
        }
    }

    /// Responses takes `reasoning: {effort}`; Chat Completions and Azure take a flat
    /// `reasoning_effort` and reject the object.
    fn additional_params(&self, history: &Option<History>) -> Option<serde_json::Value> {
        let history = history.as_ref()?;
        let base = history.build_additional_params().ok().flatten();
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        let reasoning = history
            .thinking
            .and_then(|thinking| OpenAIReasoning::of(model).effort(thinking))
            .map(|effort| match self.client {
                OpenAIClientType::OpenAI(_) => json!({ "reasoning": { "effort": effort } }),
                OpenAIClientType::OpenAIChatCompletions(_) | OpenAIClientType::Azure(_) => {
                    json!({ "reasoning_effort": effort })
                }
            });

        merge_additional_params(base, reasoning)
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use std::collections::HashMap;

    use rig::agent::MultiTurnStreamItem;
    use rig::completion::ToolDefinition;
    use rig::completion::{Chat, Message};
    use rig::message::Text;
    use rig::streaming::{StreamedAssistantContent, StreamingChat};
    use rig::tool::Tool;
    use schemars::{JsonSchema, schema_for};
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{
        history::{Content, ContentType, History, HistoryMessage, ImageUrl, MessageContent, Role},
        provider::{ModelProviderConfiguration, OpenAIConfig},
    };
    use dotenv::dotenv;

    /// Inline so vision tests don't depend on a third-party host the provider must fetch.
    const TEST_IMAGE_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKklEQVR42mO44GBAU8QwasGoBaMWjFowasGoBaMWjFowasGoBaMWDBULAFMMAEwxv0+fAAAAAElFTkSuQmCC";

    fn proxy_provider() -> ModelProvider {
        ModelProvider {
            api_surface: None,
            provider_name: "hosted:openai".to_string(),
            model_id: Some("upstream-model".to_string()),
            version: None,
            params: Some(HashMap::from([
                (
                    "api_key".to_string(),
                    serde_json::Value::String("test-token".to_string()),
                ),
                (
                    "endpoint".to_string(),
                    serde_json::Value::String("https://proxy.example/api/v1".to_string()),
                ),
                (
                    "model_id".to_string(),
                    serde_json::Value::String("bit-id".to_string()),
                ),
            ])),
        }
    }

    #[tokio::test]
    async fn hosted_proxy_constructor_uses_openai_chat_completions() {
        let provider = proxy_provider();
        let model = OpenAIModel::from_provider_chat_completions(&provider)
            .await
            .expect("hosted proxy client should build");

        assert!(matches!(
            model.client,
            OpenAIClientType::OpenAIChatCompletions(_)
        ));
        assert_eq!(model.default_model.as_deref(), Some("bit-id"));
    }

    #[tokio::test]
    async fn direct_constructor_keeps_the_openai_responses_api() {
        let provider = proxy_provider();
        let model = OpenAIModel::from_provider(&provider)
            .await
            .expect("direct OpenAI client should build");

        assert!(matches!(model.client, OpenAIClientType::OpenAI(_)));
    }

    #[tokio::test]
    async fn test_openai_model_no_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let mut history = History::new(
            model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );
        history.set_stream(false);

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, mut history_msgs) = history.extract_prompt_and_history().unwrap();
        let response: String = agent.chat(prompt, &mut history_msgs).await.unwrap();

        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_azure_openai_model_no_stream() {
        dotenv().ok();

        // Skip test if Azure LLM deployment is not configured
        let deployment_name = match std::env::var("AZURE_OPENAI_LLM_DEPLOYMENT") {
            Ok(name) => name,
            Err(_) => {
                println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
                return;
            }
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(deployment_name.clone()),
            version: Some("2024-02-15-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").unwrap();
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let mut history = History::new(
            deployment_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );
        history.set_stream(false);
        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .temperature(1.0)
            .build();

        let (prompt, mut history) = history.extract_prompt_and_history().unwrap();

        let response: String = agent.chat(prompt, &mut history).await.unwrap();
        println!("Final response: {:?}", response);
        assert!(!response.is_empty());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_openai_model_stream() {
        dotenv().ok();

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some("@preset/testing".to_string()),
            version: None,
            provider_name: "openai".to_string(),
            params: None,
        };
        let endpoint = std::env::var("OPENAI_ENDPOINT").unwrap();
        let api_key = std::env::var("OPENAI_API_KEY").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let history = History::new(
            "@preset/testing".to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;
        let mut chunks = 0;
        let mut response = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Final response: {:?}", response);
        println!("Chunks: {}", chunks);
        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn test_azure_openai_model_stream() {
        dotenv().ok();

        let deployment_name = match std::env::var("AZURE_OPENAI_LLM_DEPLOYMENT") {
            Ok(name) => name,
            Err(_) => {
                println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
                return;
            }
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(deployment_name.clone()),
            version: Some("2024-02-15-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").unwrap();
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let history = History::new(
            deployment_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage::from_string(Role::User, "Hello"),
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;
        let mut chunks = 0;
        let mut response = String::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Final response: {:?}", response);
        println!("Chunks: {}", chunks);
        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    // --- Helpers for new tests ---
    fn azure_provider_and_config() -> Option<(ModelProvider, ModelProviderConfiguration)> {
        dotenv().ok();

        let deployment_name = std::env::var("AZURE_OPENAI_LLM_DEPLOYMENT").ok()?;
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").ok()?;
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").ok()?;

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(deployment_name),
            version: Some("2024-02-15-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };
        Some((provider, config))
    }

    // ========== Rig Tool Implementations ==========

    #[derive(Deserialize, JsonSchema)]
    struct WeatherArgs {
        location: String,
        unit: String,
    }

    #[derive(Debug)]
    struct WeatherError;

    impl std::fmt::Display for WeatherError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Weather tool error")
        }
    }

    impl std::error::Error for WeatherError {}

    #[derive(Deserialize, Serialize)]
    struct WeatherTool;

    impl Tool for WeatherTool {
        const NAME: &'static str = "get_current_weather";

        type Error = WeatherError;
        type Args = WeatherArgs;
        type Output = String;

        async fn definition(&self, _prompt: String) -> ToolDefinition {
            ToolDefinition {
                name: "get_current_weather".to_string(),
                description: "Get the current weather in a given location".to_string(),
                parameters: serde_json::to_value(schema_for!(WeatherArgs))
                    .expect("Failed to serialize weather args schema"),
            }
        }

        async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
            Ok(format!(
                "The weather in {} is 22 degrees {}",
                args.location, args.unit
            ))
        }
    }

    #[derive(Deserialize, JsonSchema)]
    struct ForecastArgs {
        location: String,
        days: i32,
    }

    #[derive(Debug)]
    struct ForecastError;

    impl std::fmt::Display for ForecastError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Forecast tool error")
        }
    }

    impl std::error::Error for ForecastError {}

    #[derive(Deserialize, Serialize)]
    struct ForecastTool;

    impl Tool for ForecastTool {
        const NAME: &'static str = "get_forecast";

        type Error = ForecastError;
        type Args = ForecastArgs;
        type Output = String;

        async fn definition(&self, _prompt: String) -> ToolDefinition {
            ToolDefinition {
                name: "get_forecast".to_string(),
                description: "Get the forecast for the next N days".to_string(),
                parameters: serde_json::to_value(schema_for!(ForecastArgs))
                    .expect("Failed to serialize forecast args schema"),
            }
        }

        async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
            Ok(format!(
                "The forecast for {} for the next {} days is sunny with temperatures ranging from 18-25 degrees",
                args.location, args.days
            ))
        }
    }

    // ========== Tool Tests ==========

    #[tokio::test]
    async fn test_azure_openai_tool_call_no_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "Call the tool to get the weather for San Francisco, CA in celsius.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        // The response should contain the tool's output (weather info)
        println!("Response: {}", response);
        assert!(!response.is_empty());
        assert!(
            response.contains("San Francisco")
                || response.contains("weather")
                || response.contains("22")
        );
    }

    #[tokio::test]
    async fn test_azure_openai_tool_call_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        use futures::StreamExt;
        let mut stream = agent
            .stream_chat(
                "Please call the tool to get the weather for Berlin in celsius.",
                Vec::<Message>::new(),
            )
            .await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Streamed response: {}", response);
        assert!(!response.is_empty());
    }

    #[tokio::test]
    async fn test_azure_openai_tool_result_roundtrip() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        // This will automatically call the tool and use its result
        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "What is the weather in Paris in celsius? Use the tool.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        println!("Roundtrip response: {}", response);
        assert!(!response.is_empty());
        // Response should mention Paris or the weather info
        assert!(
            response.contains("Paris") || response.contains("weather") || response.contains("22")
        );
    }

    #[tokio::test]
    async fn test_azure_openai_vision_no_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.clone().unwrap();

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image succinctly.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, mut history_msgs) = history.extract_prompt_and_history().unwrap();

        let response: String = match agent.chat(prompt, &mut history_msgs).await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                    eprintln!("Skipping due to rate limit: {msg}");
                    return;
                }
                if msg.contains("No endpoints found that support tool use")
                    || msg.contains("404 Not Found")
                {
                    eprintln!("Skipping: tool use unsupported on route: {msg}");
                    return;
                }
                panic!("{e}");
            }
        };
        assert!(!response.is_empty());
    }

    #[tokio::test]
    async fn test_azure_openai_vision_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.clone().unwrap();

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.clone(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;

        let mut chunks = 0;
        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                        eprintln!("Skipping due to rate limit: {msg}");
                        return;
                    }
                    panic!("Stream error: {}", e);
                }
            }
        }

        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    // -------- OpenAI parity tests (tool calling, roundtrip, vision) --------

    fn openai_provider_and_config() -> (ModelProvider, ModelProviderConfiguration) {
        dotenv().ok();
        // Use a specific model that works with OpenRouter instead of @preset/testing
        // openai/gpt-4o-mini supports tools and is available via OpenRouter
        let model_id = if std::env::var("OPENAI_ENDPOINT")
            .unwrap_or_default()
            .contains("openrouter")
        {
            "openai/gpt-4o-mini".to_string()
        } else {
            "gpt-3.5-turbo".to_string()
        };

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some(model_id),
            version: None,
            provider_name: "openai".to_string(),
            params: None,
        };
        let endpoint = std::env::var("OPENAI_ENDPOINT").unwrap();
        let api_key = std::env::var("OPENAI_API_KEY").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };
        (provider, config)
    }

    #[tokio::test]
    async fn test_openai_tool_call_no_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or("@preset/testing".to_string()))
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "Call the tool to get the weather for San Francisco, CA in celsius.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        println!("Response: {}", response);
        assert!(!response.is_empty());
        assert!(
            response.contains("San Francisco")
                || response.contains("weather")
                || response.contains("22")
        );
    }

    #[tokio::test]
    async fn test_openai_tool_call_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or("@preset/testing".to_string()))
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        use futures::StreamExt;
        let mut stream = agent
            .stream_chat(
                "Please call the tool to get the weather for Berlin in celsius.",
                Vec::<Message>::new(),
            )
            .await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        println!("Streamed response: {}", response);
        assert!(!response.is_empty());
    }

    #[tokio::test]
    async fn test_openai_tool_result_roundtrip() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or("@preset/testing".to_string()))
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .build();

        let mut history = Vec::<Message>::new();
        let response: String = agent
            .chat(
                "What is the weather in Paris in celsius? Use the tool.",
                &mut history,
            )
            .await
            .expect("Failed to get response");

        println!("Roundtrip response: {}", response);
        assert!(!response.is_empty());
        assert!(
            response.contains("Paris") || response.contains("weather") || response.contains("22")
        );
    }

    #[tokio::test]
    async fn test_openai_vision_no_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        // Use vision-capable model for OpenRouter, otherwise use default
        let model_name = if std::env::var("OPENAI_ENDPOINT")
            .unwrap_or_default()
            .contains("openrouter")
        {
            "openai/gpt-4o-mini"
        } else {
            provider.model_id.as_ref().unwrap()
        };

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image succinctly.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, mut history_msgs) = history.extract_prompt_and_history().unwrap();

        let response: String = match agent.chat(prompt, &mut history_msgs).await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                    eprintln!("Skipping due to rate limit: {msg}");
                    return;
                }
                eprintln!("OpenAI vision error: {msg}");
                return;
            }
        };
        assert!(!response.is_empty());
    }

    #[tokio::test]
    async fn test_openai_vision_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        // Use vision-capable model for OpenRouter, otherwise use default
        let model_name = if std::env::var("OPENAI_ENDPOINT")
            .unwrap_or_default()
            .contains("openrouter")
        {
            "openai/gpt-4o-mini"
        } else {
            provider.model_id.as_ref().unwrap()
        };

        let image_url = TEST_IMAGE_URL;
        let history = History::new(
            model_name.to_string(),
            vec![
                HistoryMessage::from_string(Role::System, "You are a helpful assistant."),
                HistoryMessage {
                    role: Role::User,
                    content: MessageContent::Contents(vec![
                        Content::Text {
                            content_type: ContentType::Text,
                            text: "Describe the image.".to_string(),
                        },
                        Content::Image {
                            content_type: ContentType::ImageUrl,
                            image_url: ImageUrl {
                                url: image_url.to_string(),
                                detail: None,
                                media_type: None,
                                additional_params: None,
                            },
                        },
                    ]),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                },
            ],
        );

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(&model.default_model.unwrap_or(history.model.clone()))
            .build();

        let (prompt, history_msgs) = history.extract_prompt_and_history().unwrap();

        use futures::StreamExt;
        let mut stream = agent.stream_chat(prompt, history_msgs).await;

        let mut chunks = 0;
        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                    chunks += 1;
                }
                Ok(_) => {} // Ignore other stream items
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
                        eprintln!("Skipping due to rate limit: {msg}");
                        return;
                    }
                    eprintln!("OpenAI vision stream error: {msg}");
                    panic!("Stream error: {}", e);
                }
            }
        }

        assert!(!response.is_empty());
        assert!(chunks > 0);
    }

    // -------- Parallel tool-calls (both providers) --------

    #[tokio::test]
    async fn test_azure_openai_parallel_tool_calls_no_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .build();

        let prompt =
            "Call both weather and forecast tools for Berlin (3 days), return tool calls only.";
        let mut history = Vec::<Message>::new();
        let response: String = agent.chat(prompt, &mut history).await.unwrap();

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    async fn test_azure_openai_parallel_tool_calls_stream() {
        let Some((provider, config)) = azure_provider_and_config() else {
            println!("Skipping Azure LLM test: AZURE_OPENAI_LLM_DEPLOYMENT not set");
            return;
        };
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .default_max_turns(3)
            .build();

        use futures::StreamExt;
        let prompt = "Call both weather and forecast tools for Berlin (3 days).";
        let mut stream = agent.stream_chat(prompt, Vec::<Message>::new()).await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    async fn test_openai_parallel_tool_calls_no_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();
        let model_name = provider.model_id.as_ref().unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent(model_name)
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .build();

        let prompt =
            "Call both weather and forecast tools for Berlin (3 days), return tool calls only.";
        let mut history = Vec::<Message>::new();
        let response: String = agent.chat(prompt, &mut history).await.unwrap();

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    async fn test_openai_parallel_tool_calls_stream() {
        let (provider, config) = openai_provider_and_config();
        let model = OpenAIModel::new(&provider, &config).await.unwrap();

        let agent = model
            .provider()
            .await
            .unwrap()
            .inner
            .agent("@preset/testing")
            .preamble("You are a helpful assistant.")
            .tool(WeatherTool)
            .tool(ForecastTool)
            .default_max_turns(3)
            .build();

        use futures::StreamExt;
        let prompt = "Call both weather and forecast tools for Berlin (3 days).";
        let mut stream = agent.stream_chat(prompt, Vec::<Message>::new()).await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text, .. },
                ))) => {
                    response.push_str(&text);
                }
                Ok(_) => {}
                Err(e) => panic!("Stream error: {}", e),
            }
        }

        assert!(!response.is_empty());
        assert!(response.contains("Berlin") || response.contains("berlin"));
    }

    #[tokio::test]
    async fn azure_forwards_the_output_budget() {
        use crate::llm::test_support::{chat_completion, json_response, serve_once};

        let (endpoint, server) =
            serve_once(json_response(chat_completion("gpt-4o", "length", 2048))).await;
        let model = OpenAIModel::from_provider(&ModelProvider {
            api_surface: None,
            provider_name: "openai".to_string(),
            model_id: None,
            version: None,
            params: Some(HashMap::from([
                ("api_key".to_string(), serde_json::json!("test-key")),
                ("model_id".to_string(), serde_json::json!("gpt-4o")),
                ("endpoint".to_string(), serde_json::json!(endpoint)),
                ("is_azure".to_string(), serde_json::json!(true)),
            ])),
        })
        .await
        .unwrap();
        let mut history = History::new(
            "gpt-4o".to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Summarize the run.",
            )],
        );
        history.max_completion_tokens = Some(2048);

        let response = model.invoke(&history, None).await.unwrap();

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(response.choices[0].finish_reason, "length");
    }

    #[tokio::test]
    async fn azure_reasoning_deployment_gets_its_own_parameter_names() {
        use crate::llm::test_support::{chat_completion, json_response, serve_once};

        let (endpoint, server) =
            serve_once(json_response(chat_completion("o4-mini", "stop", 12))).await;
        let model = OpenAIModel::from_provider(&ModelProvider {
            api_surface: None,
            provider_name: "openai".to_string(),
            model_id: None,
            version: None,
            params: Some(HashMap::from([
                ("api_key".to_string(), serde_json::json!("test-key")),
                ("model_id".to_string(), serde_json::json!("o4-mini")),
                ("endpoint".to_string(), serde_json::json!(endpoint)),
                ("is_azure".to_string(), serde_json::json!(true)),
            ])),
        })
        .await
        .unwrap();
        let mut history = History::new(
            "o4-mini".to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Summarize the run.",
            )],
        );
        history.max_completion_tokens = Some(4096);
        history.thinking = Some(HistoryThinking::High);
        history.temperature = Some(0.2);
        history.top_p = Some(0.9);

        model.invoke(&history, None).await.unwrap();

        let body: serde_json::Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["max_completion_tokens"], 4096);
        assert_eq!(body["reasoning_effort"], "high");
        for rejected in ["max_tokens", "reasoning", "temperature", "top_p"] {
            assert!(body.get(rejected).is_none(), "{rejected} in {body}");
        }
    }

    #[test]
    fn reasoning_family_comes_from_the_model_id() {
        for (id, expected) in [
            ("gpt-4o", OpenAIReasoning::Unsupported),
            ("gpt-4.1-mini", OpenAIReasoning::Unsupported),
            ("gpt-5-chat-latest", OpenAIReasoning::Unsupported),
            ("o1-mini", OpenAIReasoning::Unsupported),
            ("o4-mini", OpenAIReasoning::OSeries),
            ("gpt-5-mini", OpenAIReasoning::Gpt5 { minor: 0 }),
            ("openai/gpt-5.4", OpenAIReasoning::Gpt5 { minor: 4 }),
            ("gpt-6-astra", OpenAIReasoning::Gpt6),
            ("openai.gpt-oss-120b-1:0", OpenAIReasoning::GptOss),
            ("flowpilot-hosted-bit", OpenAIReasoning::Unknown),
        ] {
            assert_eq!(OpenAIReasoning::of(id), expected, "{id}");
        }
        assert_eq!(
            OpenAIReasoning::Gpt5 { minor: 0 }.effort(HistoryThinking::Off),
            Some("minimal")
        );
        assert_eq!(
            OpenAIReasoning::Gpt5 { minor: 1 }.effort(HistoryThinking::Off),
            Some("none")
        );
        assert_eq!(
            OpenAIReasoning::Gpt6.effort(HistoryThinking::Off),
            Some("low")
        );
        assert_eq!(
            OpenAIReasoning::Unsupported.effort(HistoryThinking::High),
            None
        );
    }

    #[tokio::test]
    async fn reasoning_shape_follows_the_api_surface() {
        let mut history = History::new(
            "gpt-5.4".to_string(),
            vec![HistoryMessage::from_string(Role::User, "hi")],
        );
        history.thinking = Some(HistoryThinking::Mid);
        let provider = |surface| ModelProvider {
            api_surface: Some(surface),
            provider_name: "openai".to_string(),
            model_id: None,
            version: None,
            params: Some(HashMap::from([
                ("api_key".to_string(), serde_json::json!("test-key")),
                ("model_id".to_string(), serde_json::json!("gpt-5.4")),
            ])),
        };

        let responses = OpenAIModel::from_provider_with_surface(
            &provider(ModelApiSurface::Responses),
            ModelApiSurface::Responses,
        )
        .await
        .unwrap()
        .additional_params(&Some(history.clone()))
        .unwrap();
        let chat = OpenAIModel::from_provider_with_surface(
            &provider(ModelApiSurface::ChatCompletions),
            ModelApiSurface::ChatCompletions,
        )
        .await
        .unwrap()
        .additional_params(&Some(history))
        .unwrap();

        assert_eq!(
            responses["reasoning"],
            serde_json::json!({"effort": "medium"})
        );
        assert!(responses.get("reasoning_effort").is_none());
        assert_eq!(chat["reasoning_effort"], "medium");
        assert!(chat.get("reasoning").is_none());
    }
}
