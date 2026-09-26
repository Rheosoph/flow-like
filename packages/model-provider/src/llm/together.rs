use std::any::Any;

use super::{ModelLogic, OPENROUTER_ONLY, ParamDialect, merge_additional_params};
use crate::history::{History, HistoryThinking};
use crate::provider::random_provider;
use crate::{
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use serde_json::{Value, json};

const TOGETHER: ParamDialect = ParamDialect {
    provider: "together",
    renames: &[],
    unsupported: &OPENROUTER_ONLY,
    rig_owned: &[],
};

/// Together documents `reasoning_effort` (low/medium/high) for GPT-OSS; hybrid models use a
/// per-model toggle this layer cannot detect from the name.
fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    model.to_ascii_lowercase().contains("gpt-oss").then(|| {
        let effort = match thinking {
            HistoryThinking::Off | HistoryThinking::Low => "low",
            HistoryThinking::Mid => "medium",
            HistoryThinking::High => "high",
        };
        json!({ "reasoning_effort": effort })
    })
}

pub struct TogetherModel {
    client: rig::providers::together::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl TogetherModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let together_config = random_provider(&config.together_config)?;
        let api_key = together_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::together::Client::builder().api_key(&api_key);

        if let Some(endpoint) = together_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(TogetherModel {
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

        let mut builder = rig::providers::together::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(TogetherModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for TogetherModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for TogetherModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor::with_max_tokens_body_param(
            self.client.clone(),
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(TOGETHER.params(history)),
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
    fn reasoning_effort_only_for_gpt_oss() {
        assert_eq!(
            reasoning("openai/gpt-oss-120b", HistoryThinking::High),
            Some(json!({"reasoning_effort": "high"}))
        );
        assert_eq!(
            reasoning(
                "meta-llama/Llama-3.3-70B-Instruct-Turbo",
                HistoryThinking::High
            ),
            None
        );
    }
}
