use std::any::Any;

use super::{
    ModelLogic, OPENROUTER_ONLY, ParamDialect, extract_headers, merge_additional_params,
    output_budget_as_body_param,
    user_media::{self, Kind, Part, Source},
};
use crate::history::{History, HistoryThinking};
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
use rig::message::{DocumentSourceKind, Image, UserContent};
use serde_json::{Value, json};

const NAME: &str = "Ollama";

/// The image types Ollama's vision models decode.
const IMAGES: [&str; 4] = ["image/jpeg", "image/jpg", "image/png", "image/webp"];

/// Rig merges these parameters into Ollama's `options`, which skips unknown names with a
/// warning; `think` stays top-level.
const OLLAMA: ParamDialect = ParamDialect {
    provider: "ollama",
    renames: &[],
    unsupported: &[
        "user",
        "n",
        "stream_options",
        OPENROUTER_ONLY[0],
        OPENROUTER_ONLY[1],
    ],
    rig_owned: &["stream"],
};

/// `think: true` on a model without thinking returns 400, so it is sent only for known
/// thinking families. gpt-oss takes levels and cannot turn thinking off.
fn think(model: &str, thinking: HistoryThinking) -> Option<Value> {
    let model = model.to_ascii_lowercase();
    if model.contains("gpt-oss") {
        let level = match thinking {
            HistoryThinking::Off | HistoryThinking::Low => "low",
            HistoryThinking::Mid => "medium",
            HistoryThinking::High => "high",
        };
        return Some(json!({ "think": level }));
    }
    let name = model.rsplit('/').next().unwrap_or(&model);
    let thinks = name == "qwen3"
        || name.starts_with("qwen3:")
        || ["deepseek-r1", "deepseek-v3.1", "magistral", "thinking"]
            .iter()
            .any(|family| name.contains(family));
    thinks.then(|| json!({ "think": thinking != HistoryThinking::Off }))
}

/// `/api/chat` ignores a top-level `max_tokens`; the budget is `options.num_predict`. Rig's
/// converter sends base64 images and the text of user messages, and drops everything else.
fn ollama_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    user_media::deliver(
        NAME,
        model,
        output_budget_as_body_param(request, "num_predict"),
        media_part,
    )
}

fn media_part(part: &UserContent) -> Result<Part, CompletionError> {
    match part {
        UserContent::Image(image) => image_part(image),
        part => user_media::text_only(NAME, part),
    }
}

/// Ollama reads images only as base64 and never downloads a link.
fn image_part(image: &Image) -> Result<Part, CompletionError> {
    let declared = user_media::mime_of(image.media_type.as_ref());
    let source = Source::of(&image.data);
    let mime = match &source {
        Source::Inline { base64, mime } => declared
            .or(*mime)
            .or_else(|| user_media::sniff_image(base64)),
        _ => declared,
    };
    if mime.is_some_and(|mime| !IMAGES.contains(&mime)) {
        return Ok(user_media::omitted(Kind::Image, mime, &source));
    }
    match source {
        Source::Inline { .. } if matches!(image.data, DocumentSourceKind::Base64(_)) => {
            Ok(Part::Keep)
        }
        Source::Inline { base64, .. } => Ok(Part::Rig(UserContent::Image(Image {
            data: DocumentSourceKind::Base64(base64.into_owned()),
            media_type: image.media_type.clone(),
            detail: image.detail.clone(),
            additional_params: image.additional_params.clone(),
        }))),
        Source::Link(link) | Source::LocalLink(link) => {
            Err(user_media::inline_only(NAME, Kind::Image, declared, link))
        }
        source => Ok(user_media::omitted(Kind::Image, declared, &source)),
    }
}

pub struct OllamaModel {
    client: rig::providers::ollama::Client<AuthorizedHttpClient>,
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

        let mut builder = rig::providers::ollama::Client::builder()
            .api_key(rig::client::Nothing)
            .http_client(AuthorizedHttpClient::default());
        builder = builder.base_url(&endpoint);

        let client = builder.build()?;

        Ok(OllamaModel {
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
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let endpoint = params
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:11434");
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::ollama::Client::builder()
            .api_key(rig::client::Nothing)
            .http_client(AuthorizedHttpClient::for_egress(egress)?);
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
        Ok(user_media::constructor(self.client.clone(), ollama_request))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<Value> {
        let history = history.as_ref()?;
        let model = self.default_model.as_deref().unwrap_or(&history.model);
        merge_additional_params(
            Some(OLLAMA.params(history)),
            history.thinking.and_then(|thinking| think(model, thinking)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{json_response, serve_once};

    #[test]
    fn think_only_for_known_thinking_models() {
        assert_eq!(
            think("gpt-oss:20b", HistoryThinking::Mid),
            Some(json!({"think": "medium"}))
        );
        assert_eq!(
            think("qwen3:8b", HistoryThinking::High),
            Some(json!({"think": true}))
        );
        assert_eq!(
            think("deepseek-r1:14b", HistoryThinking::Off),
            Some(json!({"think": false}))
        );
        assert_eq!(think("qwen3-coder:30b", HistoryThinking::High), None);
        assert_eq!(think("llama3.2", HistoryThinking::High), None);
    }

    #[tokio::test]
    async fn output_budget_reaches_num_predict() {
        let (endpoint, server) = serve_once(json_response(json!({
            "model": "qwen3:8b",
            "created_at": "2026-09-24T00:00:00Z",
            "message": {"role": "assistant", "content": "ok"},
            "done": true,
            "done_reason": "length",
            "prompt_eval_count": 12,
            "eval_count": 64,
        })))
        .await;
        let model = OllamaModel::from_provider(
            &ModelProvider {
                provider_name: "ollama".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([
                    ("model_id".to_string(), json!("qwen3:8b")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap();
        let mut history = History::new(
            "qwen3:8b".to_string(),
            vec![crate::history::HistoryMessage::from_string(
                crate::history::Role::User,
                "Summarize the run.",
            )],
        );
        history.max_completion_tokens = Some(64);
        history.thinking = Some(HistoryThinking::High);
        history.top_p = Some(0.9);

        let response = model.invoke(&history, None).await.unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["options"]["num_predict"], 64);
        assert!(body["options"]["top_p"].as_f64().is_some());
        assert_eq!(body["think"], true);
        assert!(body["options"].get("stream").is_none());
        assert_eq!(response.choices[0].finish_reason, "length");
    }

    fn done() -> String {
        json_response(json!({
            "model": "qwen3-vl:8b",
            "created_at": "2026-10-28T00:00:00Z",
            "message": {"role": "assistant", "content": "ok"},
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 12,
            "eval_count": 1,
        }))
    }

    async fn ollama_at(endpoint: &str) -> OllamaModel {
        OllamaModel::from_provider(
            &ModelProvider {
                provider_name: "ollama".to_string(),
                model_id: None,
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
    async fn inline_images_reach_ollama_and_other_media_become_notes() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use rig::message::{DocumentMediaType, ImageMediaType};
        use user_media::tests::{PDF_BASE64, PNG_BASE64, audio, document, image, request};

        let (endpoint, server) = serve_once(done()).await;
        let model = ollama_at(&endpoint).await;
        let jpeg = "/9j/4AAQSkZJRg==";

        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("qwen3-vl:8b")
            .completion(request(vec![
                UserContent::text("Describe."),
                image(
                    DocumentSourceKind::Base64(PNG_BASE64.to_string()),
                    Some(ImageMediaType::PNG),
                ),
                image(
                    DocumentSourceKind::Raw(STANDARD.decode(jpeg).unwrap()),
                    None,
                ),
                image(
                    DocumentSourceKind::Url("data:image/webp;base64,UklGRiQAAABXRUJQ".to_string()),
                    None,
                ),
                image(
                    DocumentSourceKind::Base64("R0lGODlhAQABAA==".to_string()),
                    Some(ImageMediaType::GIF),
                ),
                document(
                    DocumentSourceKind::Base64(PDF_BASE64.to_string()),
                    Some(DocumentMediaType::PDF),
                ),
                audio(DocumentSourceKind::Base64("UklGRg==".to_string())),
            ]))
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "user",
                "content": "Describe. \
                    [attachment omitted: unsupported file type image/gif] \
                    [attachment omitted: unsupported file type application/pdf] \
                    [attachment omitted: unsupported file type audio/wav]",
                "images": [PNG_BASE64, jpeg, "UklGRiQAAABXRUJQ"],
            })
        );
    }

    #[tokio::test]
    async fn an_image_link_fails_naming_its_host_instead_of_being_dropped() {
        use user_media::tests::{PRESIGNED_PNG, assert_names_host_only, image, request};

        let (endpoint, server) = serve_once(done()).await;
        let model = ollama_at(&endpoint).await;

        let error = model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("qwen3-vl:8b")
            .completion(request(vec![
                UserContent::text("Describe."),
                image(DocumentSourceKind::Url(PRESIGNED_PNG.to_string()), None),
            ]))
            .await
            .unwrap_err()
            .to_string();

        assert_names_host_only(&error, "flow-like-content.s3.eu-central-1.amazonaws.com");
        assert!(
            error.contains("cannot be sent to Ollama as a link"),
            "{error}"
        );
        assert!(!server.is_finished());
        server.abort();
    }
}
