use std::any::Any;

use super::{ModelConstructor, ModelLogic};
use crate::{egress::ProviderEgress, provider::ModelProvider};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;

use super::llamacpp::LlamaCppClient;

pub struct LMStudioModel {
    client: LlamaCppClient,
    default_model: Option<String>,
}

impl LMStudioModel {
    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
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

        let client = LlamaCppClient::new(endpoint)
            .with_egress(egress)?
            .inline_media_only("LM Studio");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{chat_completion, json_response, serve_once};
    use crate::llm::user_media::tests::{PRESIGNED_PNG, assert_names_host_only, image, request};
    use rig::message::{DocumentSourceKind, ImageMediaType, UserContent};
    use serde_json::{Value, json};

    async fn lm_studio_at(endpoint: &str) -> LMStudioModel {
        LMStudioModel::from_provider(
            &ModelProvider {
                provider_name: "lmstudio".to_string(),
                model_id: Some("qwen2.5-vl-7b".to_string()),
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([(
                    "endpoint".to_string(),
                    json!(endpoint),
                )])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn inline_images_reach_lm_studio_as_data_urls() {
        let (endpoint, server) =
            serve_once(json_response(chat_completion("qwen2.5-vl-7b", "stop", 1))).await;
        let model = lm_studio_at(&endpoint).await;

        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("qwen2.5-vl-7b")
            .completion(request(vec![
                UserContent::text("Describe."),
                image(
                    DocumentSourceKind::Base64("/9j/4AAQSkZJRg==".to_string()),
                    Some(ImageMediaType::JPEG),
                ),
            ]))
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        let parts = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|message| message["content"].as_array())
            .flatten()
            .collect::<Vec<_>>();
        assert!(
            parts
                .iter()
                .any(|part| part["image_url"]["url"] == "data:image/jpeg;base64,/9j/4AAQSkZJRg=="),
            "{body}"
        );
    }

    #[tokio::test]
    async fn an_image_link_fails_before_reaching_lm_studio() {
        let (endpoint, server) = serve_once(json_response(json!({}))).await;
        let model = lm_studio_at(&endpoint).await;

        let error = model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("qwen2.5-vl-7b")
            .completion(request(vec![
                UserContent::text("Describe."),
                image(DocumentSourceKind::Url(PRESIGNED_PNG.to_string()), None),
            ]))
            .await
            .unwrap_err()
            .to_string();

        assert_names_host_only(&error, "flow-like-content.s3.eu-central-1.amazonaws.com");
        assert!(error.contains("cannot be sent to LM Studio"), "{error}");
        assert!(!server.is_finished());
        server.abort();
    }
}
