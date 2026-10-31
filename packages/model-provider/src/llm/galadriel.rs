use std::any::Any;

use super::{ModelLogic, output_budget_as_body_param, user_media};
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
use rig::OneOrMany;
use rig::completion::{CompletionError, CompletionRequest, Message};
use rig::message::UserContent;

const NAME: &str = "Galadriel";

/// Rig's converter sends only the first text part of a user message, so media become notes,
/// documents their text, and all text parts one part.
fn galadriel_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    let mut request = user_media::deliver(
        NAME,
        model,
        output_budget_as_body_param(request, "max_tokens"),
        |part| user_media::text_only(NAME, part),
    )?;
    for message in request.chat_history.iter_mut() {
        if let Message::User { content } = message {
            join_text(content);
        }
    }
    Ok(request)
}

/// One text part with the text of all, followed by the other parts.
fn join_text(content: &mut OneOrMany<UserContent>) {
    let (texts, others): (Vec<_>, Vec<_>) = content
        .iter()
        .cloned()
        .partition(|part| matches!(part, UserContent::Text(_)));
    if texts.len() < 2 {
        return;
    }
    let text = texts
        .iter()
        .filter_map(|part| match part {
            UserContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut joined = OneOrMany::one(UserContent::text(text));
    for part in others {
        joined.push(part);
    }
    *content = joined;
}

pub struct GaladrielModel {
    client: rig::providers::galadriel::Client<AuthorizedHttpClient>,
    default_model: Option<String>,
}

impl GaladrielModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let galadriel_config = random_provider(&config.galadriel_config)?;
        let api_key = galadriel_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::galadriel::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());

        if let Some(endpoint) = galadriel_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(GaladrielModel {
            client,
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

        let mut builder = rig::providers::galadriel::Client::builder()
            .api_key(api_key)
            .http_client(AuthorizedHttpClient::for_provider(provider, egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(GaladrielModel {
            client,
            default_model: model_id,
        })
    }
}

impl Cacheable for GaladrielModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for GaladrielModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(
            self.client.clone(),
            galadriel_request,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{chat_completion, json_response, serve_once};
    use rig::message::{DocumentMediaType, DocumentSourceKind, ImageMediaType};
    use serde_json::{Value, json};
    use user_media::tests::{PNG_BASE64, document, image, request};

    #[tokio::test]
    async fn every_part_of_a_user_message_reaches_galadriel_as_text() {
        let (endpoint, server) =
            serve_once(json_response(chat_completion("gpt-4o", "stop", 1))).await;
        let model = GaladrielModel::from_provider(
            &ModelProvider {
                provider_name: "galadriel".to_string(),
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
            .completion_model("gpt-4o")
            .completion(request(vec![
                UserContent::text("Summarise."),
                document(
                    DocumentSourceKind::String("Quarterly notes".to_string()),
                    Some(DocumentMediaType::TXT),
                ),
                image(
                    DocumentSourceKind::Base64(PNG_BASE64.to_string()),
                    Some(ImageMediaType::PNG),
                ),
            ]))
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            body["messages"][1]["content"],
            "Summarise.\nQuarterly notes\n[attachment omitted: unsupported file type image/png]"
        );
    }
}
