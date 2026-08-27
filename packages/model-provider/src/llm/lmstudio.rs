use std::any::Any;

use super::{ModelConstructor, ModelLogic};
use crate::provider::ModelProvider;
use flow_like_types::{Cacheable, Result, async_trait};

use super::llamacpp::LlamaCppClient;

pub struct LMStudioModel {
    client: LlamaCppClient,
    default_model: Option<String>,
}

impl LMStudioModel {
    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let model_id = provider.model_id.clone().or_else(|| {
            params
                .get("model_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        });
        let endpoint = params
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:1234");

        let client = LlamaCppClient::new(endpoint);

        Ok(LMStudioModel {
            client,
            default_model: model_id,
        })
    }
}

impl Cacheable for LMStudioModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for LMStudioModel {
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
