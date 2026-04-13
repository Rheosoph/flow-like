use std::any::Any;

use super::{ModelLogic, merge_additional_params};
use crate::provider::random_provider;
use crate::{
    history::History,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use flow_like_types::{Cacheable, Result, async_trait, json::json};

fn supports_reasoning_effort(model_name: Option<&str>) -> bool {
    model_name
        .map(|name| name.to_ascii_lowercase().contains("grok-3-mini"))
        .unwrap_or(false)
}

pub struct XAIModel {
    client: rig::providers::xai::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl XAIModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> flow_like_types::Result<Self> {
        let xai_config = random_provider(&config.xai_config)?;
        let api_key = xai_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::xai::Client::builder().api_key(&api_key);

        if let Some(endpoint) = xai_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(XAIModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let mut builder = rig::providers::xai::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(XAIModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for XAIModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for XAIModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<flow_like_types::Value> {
        let history = history.as_ref()?;
        let base = history.build_additional_params().ok().flatten();
        let model_name = self
            .default_model
            .as_deref()
            .or(Some(history.model.as_str()));

        if !supports_reasoning_effort(model_name) {
            return base;
        }

        let reasoning = history
            .thinking
            .and_then(|thinking| thinking.xai_reasoning_effort())
            .map(|effort| {
                json!({
                    "reasoning_effort": effort,
                })
            });

        merge_additional_params(base, reasoning)
    }
}
