use std::any::Any;

use super::{ModelLogic, OPENROUTER_ONLY, ParamDialect, extract_headers, merge_additional_params};
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

/// Groq rejects unknown fields with a 400; Rig's Groq request carries its own `stream`.
const GROQ: ParamDialect = ParamDialect {
    provider: "groq",
    renames: &[],
    unsupported: &OPENROUTER_ONLY,
    rig_owned: &["stream"],
};

/// `reasoning_effort` per Groq's reasoning docs: gpt-oss takes low/medium/high, qwen3 turns
/// reasoning off with `none`. Other models reject values outside their set.
fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    if model.contains("gpt-oss") {
        let effort = match thinking {
            HistoryThinking::Off | HistoryThinking::Low => "low",
            HistoryThinking::Mid => "medium",
            HistoryThinking::High => "high",
        };
        return Some(json!({ "reasoning_effort": effort }));
    }
    (model.contains("qwen3") && thinking == HistoryThinking::Off)
        .then(|| json!({ "reasoning_effort": "none" }))
}

pub struct GroqModel {
    client: rig::providers::groq::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl GroqModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let groq_config = random_provider(&config.groq_config)?;
        let api_key = groq_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::groq::Client::builder().api_key(&api_key);

        if let Some(endpoint) = groq_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(GroqModel {
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
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::groq::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(GroqModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for GroqModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for GroqModel {
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
            Some(GROQ.params(history)),
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
    fn reasoning_effort_only_for_models_that_document_it() {
        assert_eq!(
            reasoning("openai/gpt-oss-120b", HistoryThinking::Mid),
            Some(json!({"reasoning_effort": "medium"}))
        );
        assert_eq!(
            reasoning("qwen/qwen3-32b", HistoryThinking::Off),
            Some(json!({"reasoning_effort": "none"}))
        );
        assert_eq!(reasoning("qwen/qwen3-32b", HistoryThinking::High), None);
        assert_eq!(
            reasoning("llama-3.3-70b-versatile", HistoryThinking::High),
            None
        );
    }

    #[test]
    fn transport_flag_stays_with_rig() {
        let history = History::new("llama-3.3-70b-versatile".to_string(), Vec::new());
        assert!(GROQ.params(&history).get("stream").is_none());
    }
}
