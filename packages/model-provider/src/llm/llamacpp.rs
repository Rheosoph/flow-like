use std::any::Any;

use super::{ModelConstructor, ModelLogic};
use crate::provider::ModelProvider;
use flow_like_types::{Cacheable, Result, async_trait};

mod client;
pub use client::{CompletionModel, LlamaCppClient};

pub struct LlamaCppModel {
    client: LlamaCppClient,
    _provider: ModelProvider,
    default_model: Option<String>,
    port: u16,
}

impl LlamaCppModel {
    pub async fn new(provider: &ModelProvider, port: u16) -> flow_like_types::Result<Self> {
        let model_id = provider.model_id.clone();
        let base_url = format!("http://127.0.0.1:{}", port);

        let client = LlamaCppClient::new(&base_url);

        Ok(LlamaCppModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
            port,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let endpoint = params
            .get("endpoint")
            .and_then(|value| value.as_str())
            .unwrap_or("http://localhost:8080");
        let default_model = provider.model_id.clone().or_else(|| {
            params
                .get("model_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        });

        Ok(Self {
            client: LlamaCppClient::new(endpoint),
            _provider: provider.clone(),
            default_model,
            port: 0,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn completion_model(&self, model: &str) -> CompletionModel {
        self.client.completion_model(model)
    }
}

impl Cacheable for LlamaCppModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for LlamaCppModel {
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
