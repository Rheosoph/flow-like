use super::{ModelLogic, extract_headers, merge_additional_params};
use crate::provider::random_provider;
use crate::{
    history::History,
    llm::{CompletionClientDyn, ModelConstructor},
    provider::{ModelProvider, ModelProviderConfiguration},
};
use flow_like_types::{Cacheable, Result, async_trait, json::json};
use std::any::Any;

#[derive(Clone)]
enum OpenAIClientType {
    OpenAI(rig::providers::openai::Client),
    Azure(rig::providers::azure::Client),
}

impl OpenAIClientType {
    fn into_boxed(self) -> Box<dyn CompletionClientDyn> {
        match self {
            OpenAIClientType::OpenAI(client) => Box::new(client),
            OpenAIClientType::Azure(client) => Box::new(client),
        }
    }
}

pub struct OpenAIModel {
    client: OpenAIClientType,
    default_model: Option<String>,
}

impl OpenAIModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> flow_like_types::Result<Self> {
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
            let mut builder = rig::providers::openai::Client::builder().api_key(&api_key);
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
    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<Self> {
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

        if is_azure && endpoint.is_none() {
            return Err(flow_like_types::anyhow!(
                "Azure OpenAI requires an endpoint"
            ));
        }

        if is_azure && model_id.is_none() {
            return Err(flow_like_types::anyhow!(
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
            let mut builder = rig::providers::openai::Client::builder().api_key(api_key);
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
        Ok(ModelConstructor {
            inner: self.client.clone().into_boxed(),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<flow_like_types::Value> {
        let history = history.as_ref()?;
        let base = history.build_additional_params().ok().flatten();
        let reasoning = history.thinking.map(|thinking| {
            json!({
                "reasoning": {
                    "effort": thinking.openai_reasoning_effort(),
                }
            })
        });

        merge_additional_params(base, reasoning)
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use flow_like_types::tokio;
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
                    Text { text },
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
                    Text { text },
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
                parameters: flow_like_types::json::to_value(schema_for!(WeatherArgs))
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
                parameters: flow_like_types::json::to_value(schema_for!(ForecastArgs))
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

        let response: String = agent
            .chat(
                "Call the tool to get the weather for San Francisco, CA in celsius.",
                &mut Vec::<Message>::new(),
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
                    Text { text },
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
        let response: String = agent
            .chat(
                "What is the weather in Paris in celsius? Use the tool.",
                &mut Vec::<Message>::new(),
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

        let image_url =
            "https://upload.wikimedia.org/wikipedia/commons/thumb/3/3a/Cat03.jpg/320px-Cat03.jpg";
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

        let image_url =
            "https://upload.wikimedia.org/wikipedia/commons/thumb/3/3a/Cat03.jpg/320px-Cat03.jpg";
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
                    Text { text },
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

        let response: String = agent
            .chat(
                "Call the tool to get the weather for San Francisco, CA in celsius.",
                &mut Vec::<Message>::new(),
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
                    Text { text },
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

        let response: String = agent
            .chat(
                "What is the weather in Paris in celsius? Use the tool.",
                &mut Vec::<Message>::new(),
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

        let image_url =
            "https://upload.wikimedia.org/wikipedia/commons/thumb/3/3a/Cat03.jpg/320px-Cat03.jpg";
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

        let image_url =
            "https://upload.wikimedia.org/wikipedia/commons/thumb/3/3a/Cat03.jpg/320px-Cat03.jpg";
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
                    Text { text },
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
        let response: String = agent.chat(prompt, &mut Vec::<Message>::new()).await.unwrap();

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
            .build();

        use futures::StreamExt;
        let prompt = "Call both weather and forecast tools for Berlin (3 days).";
        let mut stream = agent.stream_chat(prompt, Vec::<Message>::new()).await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text },
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
        let response: String = agent.chat(prompt, &mut Vec::<Message>::new()).await.unwrap();

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
            .build();

        use futures::StreamExt;
        let prompt = "Call both weather and forecast tools for Berlin (3 days).";
        let mut stream = agent.stream_chat(prompt, Vec::<Message>::new()).await;

        let mut response = String::new();
        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    Text { text },
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
}
