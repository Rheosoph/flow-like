use std::any::Any;

use super::ModelLogic;
use crate::provider::random_provider;
use crate::{
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;

pub struct MozillaModel {
    client: rig::providers::openai::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl MozillaModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let mozilla_config = random_provider(&config.mozilla_config)?;
        let api_key = mozilla_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let endpoint = mozilla_config
            .endpoint
            .as_deref()
            .unwrap_or("http://localhost:8000/v1");

        let client = rig::providers::openai::Client::builder()
            .api_key(&api_key)
            .base_url(endpoint)
            .build()?;

        Ok(MozillaModel {
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

        let endpoint = params
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:8000/v1");

        let client = rig::providers::openai::Client::builder()
            .api_key(api_key)
            .base_url(endpoint)
            .build()?;

        Ok(MozillaModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for MozillaModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for MozillaModel {
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
