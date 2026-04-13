use std::any::Any;

use super::{ModelLogic, extract_headers};
use crate::provider::random_provider;
use crate::{
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use flow_like_types::{Cacheable, Result, async_trait};

pub struct AnthropicModel {
    client: rig::providers::anthropic::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl AnthropicModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> flow_like_types::Result<Self> {
        let anthropic_config = random_provider(&config.anthropic_config)?;
        let api_key = anthropic_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::anthropic::Client::builder().api_key(&api_key);
        if let Some(endpoint) = anthropic_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        if let Some(beta) = anthropic_config.beta.as_deref() {
            builder = builder.anthropic_beta(beta);
        }

        if let Some(version) = anthropic_config.version.as_deref() {
            builder = builder.anthropic_version(version);
        }

        let client = builder.build()?;

        Ok(AnthropicModel {
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
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::anthropic::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }
        if let Some(beta) = params.get("beta").and_then(|v| v.as_str()) {
            builder = builder.anthropic_beta(beta);
        }

        if let Some(version) = params.get("version").and_then(|v| v.as_str()) {
            builder = builder.anthropic_version(version);
        }

        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(AnthropicModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for AnthropicModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for AnthropicModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }
}
