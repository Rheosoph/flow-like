use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, merge_additional_params,
    output_budget_as_body_param,
    user_media::{self, Doc, Kind, MediaSplice, PDF, Part, Source},
};
use crate::history::{History, HistoryThinking};
use crate::provider::random_provider;
use crate::{
    egress::ProviderEgress,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::completion::{CompletionError, CompletionRequest};
use rig::message::{Audio, Document, UserContent};
use serde_json::{Value, json};

const NAME: &str = "Mistral";

/// Mistral's schema forbids extra fields (422 "Extra inputs are not permitted").
const MISTRAL: ParamDialect = ParamDialect {
    provider: "mistral",
    renames: &[("seed", "random_seed")],
    unsupported: &[
        "user",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &[],
};

/// Adjustable reasoning is documented for these models only, and they accept `none` or `high`.
fn reasoning(model: &str, thinking: HistoryThinking) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    let adjustable = model == "mistral-small-latest" || model.contains("mistral-medium-3-5");
    adjustable.then(|| {
        json!({ "reasoning_effort": if thinking == HistoryThinking::Off { "none" } else { "high" } })
    })
}

/// Rig's converter sends only the text of user messages; the media parts Mistral reads travel
/// past it as `image_url`, `document_url` and `input_audio` chunks.
fn mistral_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    user_media::deliver(
        NAME,
        model,
        output_budget_as_body_param(request, "max_tokens"),
        |part| media_part(model, part),
    )
}

fn media_part(model: &str, part: &UserContent) -> Result<Part, CompletionError> {
    match part {
        UserContent::Image(image) => user_media::web_image(NAME, image, true),
        UserContent::Document(document) => pdf_part(document),
        UserContent::Audio(audio) if hears_audio(model) => audio_part(audio),
        part => user_media::text_only(NAME, part),
    }
}

/// Mistral reads PDFs from public links and `data:` URLs; other documents are read as text.
fn pdf_part(document: &Document) -> Result<Part, CompletionError> {
    let url = match Doc::of(document) {
        Doc::File {
            mime: Some(PDF),
            source: Source::Link(link),
        } => link.to_string(),
        Doc::File {
            mime: Some(PDF),
            source: Source::LocalLink(link),
        } => {
            return Err(user_media::unreachable(
                NAME,
                Kind::Document,
                Some(PDF),
                link,
            ));
        }
        Doc::File {
            mime: Some(PDF),
            source: Source::Inline { base64, .. },
        } => format!("data:{PDF};base64,{base64}"),
        doc => return doc.read_as_text(NAME, document),
    };
    Ok(Part::Wire(
        json!({ "type": "document_url", "document_url": url }),
    ))
}

/// Audio chat is documented for the Voxtral models.
fn hears_audio(model: &str) -> bool {
    model.to_ascii_lowercase().contains("voxtral")
}

/// `input_audio` takes the base64 of the file.
fn audio_part(audio: &Audio) -> Result<Part, CompletionError> {
    let mime = user_media::mime_of(audio.media_type.as_ref());
    match Source::of(&audio.data) {
        Source::Inline { base64, .. } => Ok(Part::Wire(
            json!({ "type": "input_audio", "input_audio": base64 }),
        )),
        Source::Link(link) | Source::LocalLink(link) => {
            Err(user_media::inline_only(NAME, Kind::Audio, mime, link))
        }
        source => Ok(user_media::omitted(Kind::Audio, mime, &source)),
    }
}

pub struct MistralModel {
    client: rig::providers::mistral::Client<MediaSplice>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl MistralModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let mistral_config = random_provider(&config.mistral_config)?;
        let api_key = mistral_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::mistral::Client::builder()
            .api_key(&api_key)
            .http_client(MediaSplice::default());

        if let Some(endpoint) = mistral_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MistralModel {
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

        let mut builder = rig::providers::mistral::Client::builder()
            .api_key(api_key)
            .http_client(MediaSplice::for_egress(egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(MistralModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for MistralModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for MistralModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(
            self.client.clone(),
            mistral_request,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(MISTRAL.params(history)),
            history
                .thinking
                .and_then(|thinking| reasoning(model, thinking)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{json_response, serve_once};
    use rig::message::{DocumentMediaType, DocumentSourceKind, ImageDetail, ImageMediaType};
    use user_media::tests::{
        PDF_BASE64, PNG_BASE64, PRESIGNED_PDF, PRESIGNED_PNG, assert_names_host_only, audio,
        document, image, request,
    };

    #[test]
    fn history_fields_use_mistral_names() {
        let mut history = History::new("mistral-large-latest".to_string(), Vec::new());
        history.seed = Some(7);
        history.user = Some("user-1".into());
        history.top_p = Some(0.9);
        let params = MISTRAL.params(&history);

        assert_eq!(params["random_seed"], 7);
        assert!(params.get("top_p").is_some());
        for rejected in ["seed", "user"] {
            assert!(params.get(rejected).is_none(), "{rejected} in {params}");
        }
    }

    #[test]
    fn reasoning_effort_only_for_adjustable_models() {
        assert_eq!(
            reasoning("mistral-small-latest", HistoryThinking::Off),
            Some(json!({"reasoning_effort": "none"}))
        );
        assert_eq!(
            reasoning("mistral-medium-3-5", HistoryThinking::Mid),
            Some(json!({"reasoning_effort": "high"}))
        );
        assert_eq!(
            reasoning("mistral-large-latest", HistoryThinking::High),
            None
        );
    }

    async fn mistral_at(endpoint: &str) -> MistralModel {
        MistralModel::from_provider(
            &ModelProvider {
                provider_name: "mistral".to_string(),
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
        .unwrap()
    }

    fn completion() -> Value {
        json!({
            "id": "cmpl-test",
            "object": "chat.completion",
            "created": 1,
            "model": "mistral-small-latest",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 1, "total_tokens": 13},
        })
    }

    #[tokio::test]
    async fn images_pdfs_and_audio_reach_mistral_as_content_chunks() {
        let (endpoint, server) = serve_once(json_response(completion())).await;
        let model = mistral_at(&endpoint).await;
        let mut png = image(
            DocumentSourceKind::Base64(PNG_BASE64.to_string()),
            Some(ImageMediaType::PNG),
        );
        if let UserContent::Image(image) = &mut png {
            image.detail = Some(ImageDetail::High);
        }
        let request = request(vec![
            UserContent::text("Compare them."),
            png,
            image(DocumentSourceKind::Url(PRESIGNED_PNG.to_string()), None),
            document(
                DocumentSourceKind::Base64(PDF_BASE64.to_string()),
                Some(DocumentMediaType::PDF),
            ),
            document(
                DocumentSourceKind::Url(PRESIGNED_PDF.to_string()),
                Some(DocumentMediaType::PDF),
            ),
            document(
                DocumentSourceKind::String("a,b\n1,2".to_string()),
                Some(DocumentMediaType::CSV),
            ),
            audio(DocumentSourceKind::Base64("UklGRg==".to_string())),
        ]);

        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("voxtral-small-latest")
            .completion(request)
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Compare them."},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{PNG_BASE64}"), "detail": "high"}},
                    {"type": "image_url", "image_url": {"url": PRESIGNED_PNG}},
                    {"type": "document_url", "document_url": format!("data:application/pdf;base64,{PDF_BASE64}")},
                    {"type": "document_url", "document_url": PRESIGNED_PDF},
                    {"type": "text", "text": "a,b\n1,2"},
                    {"type": "input_audio", "input_audio": "UklGRg=="},
                ],
            })
        );
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn text_only_requests_keep_rigs_shape() {
        let (endpoint, server) = serve_once(json_response(completion())).await;
        let model = mistral_at(&endpoint).await;

        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("mistral-small-latest")
            .completion(request(vec![
                UserContent::text("Hello."),
                audio(DocumentSourceKind::Base64("UklGRg==".to_string())),
            ]))
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            body["messages"][1],
            json!({"role": "user", "content": "Hello."})
        );
        assert_eq!(
            body["messages"][2],
            json!({
                "role": "user",
                "content": "[attachment omitted: unsupported file type audio/wav]",
            })
        );
    }

    #[tokio::test]
    async fn a_local_link_fails_before_anything_is_sent() {
        let (endpoint, server) = serve_once(json_response(completion())).await;
        let model = mistral_at(&endpoint).await;

        let error = model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("mistral-small-latest")
            .completion(request(vec![image(
                DocumentSourceKind::Url(
                    "asset://localhost/%2FUsers%2Fme%2Ftmp%2Fscan.png".to_string(),
                ),
                Some(ImageMediaType::PNG),
            )]))
            .await
            .unwrap_err()
            .to_string();

        assert_names_host_only(&error, "localhost");
        assert!(error.contains("cannot be sent to Mistral"), "{error}");
        assert!(!server.is_finished());
        server.abort();
    }
}
