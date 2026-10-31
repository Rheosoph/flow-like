use std::any::Any;

use super::{ModelLogic, user_media};
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
use rig::completion::{CompletionError, CompletionRequest};

const NAME: &str = "Mira";

/// Rig's converter sends user text only, so media become notes and documents their text.
fn mira_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    user_media::deliver(NAME, model, request, |part| {
        user_media::text_only(NAME, part)
    })
}

pub struct MiraModel {
    client: rig::providers::mira::Client<AuthorizedHttpClient>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl MiraModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let mira_config = random_provider(&config.mira_config)?;
        let api_key = mira_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::mira::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());

        if let Some(endpoint) = mira_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MiraModel {
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

        let mut builder = rig::providers::mira::Client::builder()
            .api_key(api_key)
            .http_client(AuthorizedHttpClient::for_provider(provider, egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MiraModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for MiraModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for MiraModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(self.client.clone(), mira_request))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{chat_completion, json_response, serve_once};
    use rig::message::{DocumentMediaType, DocumentSourceKind, UserContent};
    use serde_json::{Value, json};
    use user_media::tests::{PDF_BASE64, PRESIGNED_PNG, document, image, request};

    #[tokio::test]
    async fn media_reach_mira_as_notes_and_documents_as_text() {
        let (endpoint, server) =
            serve_once(json_response(chat_completion("mira-model", "stop", 1))).await;
        let model = MiraModel::from_provider(
            &ModelProvider {
                provider_name: "mira".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([
                    ("api_key".to_string(), json!("key")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap();

        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("mira-model")
            .completion(request(vec![
                UserContent::text("Summarise."),
                image(DocumentSourceKind::Url(PRESIGNED_PNG.to_string()), None),
                document(
                    DocumentSourceKind::Base64(PDF_BASE64.to_string()),
                    Some(DocumentMediaType::PDF),
                ),
                document(
                    DocumentSourceKind::String("Quarterly notes".to_string()),
                    Some(DocumentMediaType::TXT),
                ),
            ]))
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            body["messages"][1]["content"],
            "Summarise.\n\
             [attachment omitted: unsupported file type image/png]\n\
             [attachment omitted: unsupported file type application/pdf]\n\
             Quarterly notes"
        );
    }
}
