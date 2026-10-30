use std::any::Any;

use super::{
    ModelLogic,
    media::{self, MediaDialect},
    output_budget_as_body_param, user_media,
};
use crate::provider::random_provider;
use crate::{
    authorization::AuthorizedHttpClient,
    egress::ProviderEgress,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;

pub struct HyperbolicModel {
    client: rig::providers::hyperbolic::Client<AuthorizedHttpClient>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl HyperbolicModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let hyperbolic_config = random_provider(&config.hyperbolic_config)?;
        let api_key = hyperbolic_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::hyperbolic::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());

        if let Some(endpoint) = hyperbolic_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(HyperbolicModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let mut builder = rig::providers::hyperbolic::Client::builder()
            .api_key(api_key)
            .http_client(AuthorizedHttpClient::for_provider(provider, egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(HyperbolicModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for HyperbolicModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for HyperbolicModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(
            self.client.clone(),
            |model, request| {
                media::retain_supported(
                    MediaDialect::Hyperbolic,
                    model,
                    output_budget_as_body_param(request, "max_tokens"),
                )
            },
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }
}
