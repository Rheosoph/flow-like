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

/// Mistral's schema forbids extra fields (422 "Extra inputs are not permitted").
const MISTRAL: ParamDialect = ParamDialect {
    provider: "mistral",
    renames: &[("seed", "random_seed")],
    unsupported: &[
        "user",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &[],
};

/// Adjustable reasoning is documented for these models only, and they accept `none` or `high`.
fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    let adjustable = model == "mistral-small-latest" || model.contains("mistral-medium-3-5");
    adjustable.then(|| {
        json!({ "reasoning_effort": if thinking == HistoryThinking::Off { "none" } else { "high" } })
    })
}

pub struct MistralModel {
    client: rig::providers::mistral::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl MistralModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let mistral_config = random_provider(&config.mistral_config)?;
        let api_key = mistral_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::mistral::Client::builder().api_key(&api_key);

        if let Some(endpoint) = mistral_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MistralModel {
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

        let mut builder = rig::providers::mistral::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MistralModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for MistralModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for MistralModel {
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
            Some(MISTRAL.params(history)),
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
    fn history_fields_use_mistral_names() {
        let mut history = History::new("mistral-large-latest".to_string(), Vec::new());
        history.seed = Some(7);
        history.user = Some("user-1".into());
        history.top_p = Some(0.9);
        let params = MISTRAL.params(&history);

        assert_eq!(params["random_seed"], 7);
        assert!(params.get("top_p").is_some());
        for rejected in ["seed", "user"] {
            assert!(params.get(rejected).is_none(), "{rejected} in {params}");
        }
    }

    #[test]
    fn reasoning_effort_only_for_adjustable_models() {
        assert_eq!(
            reasoning("mistral-small-latest", HistoryThinking::Off),
            Some(json!({"reasoning_effort": "none"}))
        );
        assert_eq!(
            reasoning("mistral-medium-3-5", HistoryThinking::Mid),
            Some(json!({"reasoning_effort": "high"}))
        );
        assert_eq!(
            reasoning("mistral-large-latest", HistoryThinking::High),
            None
        );
    }
}
