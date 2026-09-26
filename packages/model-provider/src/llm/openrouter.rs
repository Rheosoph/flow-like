use std::{any::Any, sync::Arc};

use super::{ModelLogic, UsageReportingMode, extract_headers, merge_additional_params};
use crate::authorization::AuthorizedHttpClient;
use crate::provider::random_provider;
use crate::{
    history::History,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use flow_like_types_contracts::authorization::{RequestAuthorizer, ResourceAudience};
use serde_json::json;

pub struct OpenRouterModel {
    client: rig::providers::openrouter::Client<AuthorizedHttpClient>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl OpenRouterModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let openrouter_config = random_provider(&config.openrouter_config)?;
        let api_key = openrouter_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::openrouter::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());

        if let Some(endpoint) = openrouter_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(OpenRouterModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> anyhow::Result<Self> {
        Self::from_provider_with_authorizer(provider, None).await
    }

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

        let endpoint = params.get("endpoint").and_then(|v| v.as_str());
        let custom_headers = extract_headers(&params);

        let http_client = match authorizer {
            Some(authorizer) => AuthorizedHttpClient::new(
                authorizer,
                ResourceAudience::HostedModels,
                endpoint.ok_or_else(|| {
                    anyhow::anyhow!("Hosted authorization requires an explicit endpoint")
                })?,
            )?,
            None => AuthorizedHttpClient::default(),
        };
        let mut builder = rig::providers::openrouter::Client::builder()
            .api_key(api_key)
            .http_client(http_client);
        if let Some(endpoint) = endpoint {
            builder = builder.base_url(endpoint);
        }
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(OpenRouterModel {
            client,
            default_model: model_id.clone(),
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for OpenRouterModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for OpenRouterModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_max_tokens_body_param(
            self.client.clone(),
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn usage_reporting(&self) -> UsageReportingMode {
        UsageReportingMode::OpenRouterUsageInclude
    }

    fn additional_params(&self, history: &Option<History>) -> Option<serde_json::Value> {
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
mod tests {
    use super::*;
    use crate::history::{HistoryMessage, Role};
    use crate::llm::LLMCallback;
    use crate::llm::test_support::{chat_completion, json_response, serve_once, sse_response};
    use crate::response_chunk::ResponseChunk;
    use rig::completion::Completion;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const MODEL: &str = "anthropic/claude-sonnet-4.5";
    const BUDGET: u32 = 100_000;

    fn stream_chunk(delta: Value, finish_reason: Value) -> Value {
        json!({
            "id": "gen-stream",
            "model": MODEL,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
        })
    }

    fn stream_usage(completion_tokens: u32) -> Value {
        json!({
            "id": "gen-stream",
            "model": MODEL,
            "choices": [],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": completion_tokens,
                "total_tokens": completion_tokens + 12,
            },
        })
    }

    async fn model(endpoint: &str) -> OpenRouterModel {
        OpenRouterModel::from_provider(&ModelProvider {
            provider_name: "openrouter".to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: Some(HashMap::from([
                ("api_key".to_string(), json!("test-key")),
                ("model_id".to_string(), json!(MODEL)),
                ("endpoint".to_string(), json!(endpoint)),
            ])),
        })
        .await
        .unwrap()
    }

    fn budgeted_history() -> History {
        let mut history = History::new(
            MODEL.to_string(),
            vec![HistoryMessage::from_string(
                Role::User,
                "Plan the next step, then call finalize.",
            )],
        );
        history.max_completion_tokens = Some(BUDGET);
        history
    }

    fn recording_callback() -> (LLMCallback, Arc<Mutex<Vec<ResponseChunk>>>) {
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let sink = chunks.clone();
        let callback: LLMCallback = Arc::new(move |chunk| {
            sink.lock().unwrap().push(chunk);
            Box::pin(async { Ok(()) })
        });
        (callback, chunks)
    }

    fn wire_body(body: &str) -> Value {
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            body["max_tokens"], BUDGET,
            "History.max_completion_tokens must reach the OpenRouter request body"
        );
        body
    }

    #[tokio::test]
    async fn non_streaming_forwards_the_budget_and_reports_upstream_truncation() {
        let (endpoint, server) = serve_once(json_response(json!({
            "id": "gen-truncated",
            "object": "chat.completion",
            "created": 1,
            "model": MODEL,
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "native_finish_reason": "max_tokens",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning": "Still weighing which element to click",
                },
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4096, "total_tokens": 4108},
        })))
        .await;
        let (callback, chunks) = recording_callback();
        let mut history = budgeted_history();
        history.stream = Some(false);

        let response = model(&endpoint)
            .await
            .invoke(&history, Some(callback))
            .await
            .unwrap();

        assert_eq!(wire_body(&server.await.unwrap())["stream"], false);
        assert_eq!(response.choices[0].finish_reason, "length");
        assert!(response.choices[0].message.tool_calls.is_empty());
        assert_eq!(
            chunks
                .lock()
                .unwrap()
                .last()
                .and_then(ResponseChunk::finish_reason)
                .as_deref(),
            Some("length")
        );
    }

    #[tokio::test]
    async fn streaming_forwards_the_budget_and_reports_exhaustion_as_length() {
        let (endpoint, server) = serve_once(sse_response(&[
            stream_chunk(
                json!({"role": "assistant", "reasoning": "Still weighing"}),
                Value::Null,
            ),
            stream_chunk(
                json!({"reasoning": " which element to click"}),
                json!("length"),
            ),
            stream_usage(BUDGET),
        ]))
        .await;
        let (callback, chunks) = recording_callback();

        let response = model(&endpoint)
            .await
            .invoke(&budgeted_history(), Some(callback))
            .await
            .unwrap();

        assert_eq!(wire_body(&server.await.unwrap())["stream"], true);
        assert_eq!(response.choices[0].finish_reason, "length");
        assert_eq!(
            response.choices[0].message.reasoning.as_deref(),
            Some("Still weighing which element to click")
        );
        assert_eq!(
            chunks
                .lock()
                .unwrap()
                .last()
                .and_then(ResponseChunk::finish_reason)
                .as_deref(),
            Some("length")
        );
    }

    #[tokio::test]
    async fn streaming_within_the_budget_reports_stop() {
        let (endpoint, server) = serve_once(sse_response(&[
            stream_chunk(
                json!({"role": "assistant", "content": "Done."}),
                json!("stop"),
            ),
            stream_usage(42),
        ]))
        .await;
        let (callback, _) = recording_callback();

        let response = model(&endpoint)
            .await
            .invoke(&budgeted_history(), Some(callback))
            .await
            .unwrap();

        wire_body(&server.await.unwrap());
        assert_eq!(response.choices[0].finish_reason, "stop");
        assert_eq!(response.content().as_deref(), Some("Done."));
    }

    #[tokio::test]
    async fn agent_requests_forward_the_budget_without_forcing_a_stream() {
        let (endpoint, server) = serve_once(json_response(chat_completion(MODEL, "stop", 1))).await;
        let model = model(&endpoint).await;
        let mut history = budgeted_history();
        history.temperature = Some(0.25);
        assert_eq!(history.stream, Some(true));
        let settings = model.agent_settings(&Some(history)).unwrap();
        let agent = model
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent(MODEL)
            .additional_params(settings.params.unwrap())
            .temperature(settings.temperature.unwrap())
            .max_tokens(settings.max_tokens.unwrap())
            .build();

        let response = agent
            .completion(
                "Plan the next step.",
                Vec::<rig::completion::Message>::new(),
            )
            .await
            .unwrap()
            .send()
            .await
            .unwrap();

        let body = wire_body(&server.await.unwrap());
        assert!(
            body.get("stream").is_none(),
            "a non-streaming agent call must not ask for SSE: {body}"
        );
        assert_eq!(body["usage"]["include"], true);
        assert_eq!(body["temperature"], 0.25);
        assert_eq!(response.raw_response.finish_reason.as_deref(), Some("stop"));
    }
}
