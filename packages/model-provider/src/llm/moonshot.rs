use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, drop_body_param, drop_temperature,
    merge_additional_params, unforce_tool_choice,
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
use rig::message::ToolChoice;
use serde_json::{Value, json};

const MOONSHOT: ParamDialect = ParamDialect {
    provider: "moonshot",
    renames: &[],
    unsupported: &OPENROUTER_ONLY,
    rig_owned: &[],
};

/// Kimi K2.6+ fix these parameters; any explicit value other than the fixed one errors.
fn has_fixed_sampling(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    ["kimi-k2.6", "kimi-k2.7", "kimi-k3"]
        .iter()
        .any(|family| model.contains(family))
}

/// K3 takes `reasoning_effort` (low/high/max) and no `thinking`; K2.6 toggles `thinking`
/// and thinks by default; K2.7-code accepts only its default thinking configuration.
fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    if model.contains("kimi-k3") {
        let effort = match thinking {
            HistoryThinking::Off | HistoryThinking::Low => "low",
            HistoryThinking::Mid | HistoryThinking::High => "high",
        };
        return Some(json!({ "reasoning_effort": effort }));
    }
    (model.contains("kimi-k2.6") && thinking == HistoryThinking::Off)
        .then(|| json!({ "thinking": { "type": "disabled" } }))
}

fn moonshot_constraints(model: &str, mut request: CompletionRequest) -> CompletionRequest {
    if model.to_ascii_lowercase().contains("kimi-k2")
        && matches!(request.tool_choice, Some(ToolChoice::Required))
    {
        unforce_tool_choice(&mut request, model);
    }
    if has_fixed_sampling(model) {
        drop_temperature(&mut request, model, "fixed by this model");
        for key in ["top_p", "n", "presence_penalty", "frequency_penalty"] {
            drop_body_param(&mut request, key, model, "fixed by this model");
        }
    }
    request
}

pub struct MoonshotModel {
    client: rig::providers::moonshot::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl MoonshotModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let moonshot_config = random_provider(&config.moonshot_config)?;
        let api_key = moonshot_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::moonshot::Client::builder().api_key(&api_key);

        if let Some(endpoint) = moonshot_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MoonshotModel {
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

        let mut builder = rig::providers::moonshot::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MoonshotModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for MoonshotModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for MoonshotModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_request_fixup(
            self.client.clone(),
            moonshot_constraints,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(MOONSHOT.params(history)),
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
    fn kimi_thinking_controls_differ_per_generation() {
        assert_eq!(
            reasoning("kimi-k3", HistoryThinking::Mid),
            Some(json!({"reasoning_effort": "high"}))
        );
        assert_eq!(
            reasoning("kimi-k2.6", HistoryThinking::Off),
            Some(json!({"thinking": {"type": "disabled"}}))
        );
        assert_eq!(reasoning("kimi-k2.6", HistoryThinking::High), None);
        assert_eq!(reasoning("kimi-k2.7-code", HistoryThinking::Off), None);
    }

    #[test]
    fn fixed_sampling_is_not_sent() {
        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: Some(0.3),
            max_tokens: None,
            tool_choice: None,
            additional_params: Some(json!({"top_p": 0.8, "stop": ["END"]})),
            output_schema: None,
        };

        let fixed = moonshot_constraints("kimi-k2.6", request);

        assert_eq!(fixed.temperature, None);
        let params = fixed.additional_params.unwrap();
        assert!(params.get("top_p").is_none());
        assert_eq!(params["stop"], json!(["END"]));
    }

    #[test]
    fn required_tool_choice_becomes_an_instruction_on_k2() {
        let request = CompletionRequest {
            model: None,
            preamble: Some("Plan the next step.".into()),
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: Some(ToolChoice::Required),
            additional_params: None,
            output_schema: None,
        };

        let fixed = moonshot_constraints("kimi-k2.6", request);

        assert!(matches!(fixed.tool_choice, Some(ToolChoice::Auto)));
        assert_eq!(
            fixed.preamble.as_deref(),
            Some("Plan the next step.\n\nRespond by calling one of the provided tools.")
        );
    }
}
