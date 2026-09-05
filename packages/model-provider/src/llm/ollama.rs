use std::any::Any;

use super::{ModelLogic, extract_headers};
use crate::provider::random_provider;
use crate::{
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;

pub struct OllamaModel {
    client: rig::providers::ollama::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl OllamaModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let ollama_config = random_provider(&config.ollama_config)?;
        let model_id = provider.model_id.clone();
        let endpoint = ollama_config
            .endpoint
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let mut builder = rig::providers::ollama::Client::builder().api_key(rig::client::Nothing);
        builder = builder.base_url(&endpoint);

        let client = builder.build()?;

        Ok(OllamaModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let endpoint = params
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:11434");
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::ollama::Client::builder().api_key(rig::client::Nothing);
        builder = builder.base_url(endpoint);
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }

        let client = builder.build()?;

        Ok(OllamaModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for OllamaModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for OllamaModel {
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
