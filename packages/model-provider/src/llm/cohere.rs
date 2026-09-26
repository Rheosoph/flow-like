use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, body_params, merge_additional_params,
    output_budget_as_body_param,
};
use crate::history::{History, HistoryThinking};
use crate::provider::random_provider;
use crate::{
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::completion::CompletionRequest;
use serde_json::{Value, json};

/// Cohere v2 rejects unknown fields and names its sampling fields differently.
const COHERE: ParamDialect = ParamDialect {
    provider: "cohere",
    renames: &[("top_p", "p"), ("stop", "stop_sequences")],
    unsupported: &[
        "user",
        "n",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &[],
};

/// Cohere recommends leaving at least 1K tokens of `max_tokens` for the response.
const RESPONSE_RESERVE: u64 = 1024;

fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    if !model.to_ascii_lowercase().contains("reasoning") {
        return None;
    }
    let token_budget = match thinking {
        HistoryThinking::Off => return Some(json!({ "thinking": { "type": "disabled" } })),
        HistoryThinking::Low => 1024,
        HistoryThinking::Mid => 4096,
        HistoryThinking::High => 16_384,
    };
    Some(json!({ "thinking": { "type": "enabled", "token_budget": token_budget } }))
}

fn cohere_constraints(model: &str, request: CompletionRequest) -> CompletionRequest {
    let mut request = output_budget_as_body_param(request, "max_tokens");
    let Some(max_tokens) = request.max_tokens else {
        return request;
    };
    let params = body_params(&mut request);
    let budget = params
        .get("thinking")
        .and_then(|thinking| thinking.get("token_budget"))
        .and_then(Value::as_u64);
    if let Some(budget) = budget
        && budget.saturating_add(RESPONSE_RESERVE) > max_tokens
        && let Some(thinking) = params.get_mut("thinking").and_then(Value::as_object_mut)
    {
        thinking.remove("token_budget");
        tracing::warn!(
            model,
            budget,
            max_tokens,
            "Thinking budget leaves no room for the response; using the model's default budget"
        );
    }
    request
}

pub struct CohereModel {
    client: rig::providers::cohere::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl CohereModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let cohere_config = random_provider(&config.cohere_config)?;
        let api_key = cohere_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::cohere::Client::builder().api_key(&api_key);

        if let Some(endpoint) = cohere_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(CohereModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let mut builder = rig::providers::cohere::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(CohereModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for CohereModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for CohereModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_request_fixup(
            self.client.clone(),
            cohere_constraints,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(COHERE.params(history)),
            history
                .thinking
                .and_then(|thinking| reasoning(model, thinking)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_fields_use_cohere_names() {
        let mut history = History::new("command-a-03-2025".to_string(), Vec::new());
        history.top_p = Some(0.9);
        history.stop = Some(vec!["END".into()]);
        history.user = Some("user-1".into());
        let params = COHERE.params(&history);

        assert_eq!(params["stop_sequences"], json!(["END"]));
        assert!(params["p"].as_f64().is_some());
        for rejected in ["top_p", "stop", "user"] {
            assert!(params.get(rejected).is_none(), "{rejected} in {params}");
        }
    }

    #[test]
    fn thinking_budget_leaves_room_for_the_response() {
        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: Some(4096),
            tool_choice: None,
            additional_params: reasoning("command-a-reasoning-08-2025", HistoryThinking::High),
            output_schema: None,
        };

        let params = cohere_constraints("command-a-reasoning-08-2025", request)
            .additional_params
            .unwrap();

        assert_eq!(params["thinking"], json!({"type": "enabled"}));
        assert_eq!(params["max_tokens"], 4096);
        assert_eq!(reasoning("command-a-03-2025", HistoryThinking::High), None);
    }
}
