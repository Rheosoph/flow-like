use std::any::Any;

use super::{ModelLogic, UsageReportingMode, extract_headers, merge_additional_params};
use crate::provider::random_provider;
use crate::{
    history::History,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use serde_json::json;

pub struct OpenRouterModel {
    client: rig::providers::openrouter::Client,
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

        let mut builder = rig::providers::openrouter::Client::builder().api_key(&api_key);

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
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let endpoint = params.get("endpoint").and_then(|v| v.as_str());
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::openrouter::Client::builder().api_key(api_key);
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
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
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
