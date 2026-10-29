use std::any::Any;

use super::{
    ModelLogic, body_params, merge_additional_params,
    openai::{carries_media_link, drop_stored_reasoning, user_parts},
    user_media::{self, Kind, Source},
};
use crate::provider::random_provider;
use crate::{
    authorization::AuthorizedHttpClient,
    egress::ProviderEgress,
    history::History,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::completion::{CompletionError, CompletionRequest, Message as RigMessage};
use rig::message::{Document, Image, UserContent};
use serde_json::{Value, json};

const NAME: &str = "xAI";

fn supports_reasoning_effort(model_name: Option<&str>) -> bool {
    model_name
        .map(|name| name.to_ascii_lowercase().contains("grok-3-mini"))
        .unwrap_or(false)
}

/// xAI keeps a stored request for 30 days, and a media link in it may be a presigned credential.
/// Its docs also advise against storing requests with images, which may fail otherwise.
fn xai_constraints(
    _model: &str,
    mut request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    let images = user_parts(&request).any(|part| matches!(part, UserContent::Image(_)));
    if images || carries_media_link(&request) {
        body_params(&mut request).insert("store".to_string(), Value::Bool(false));
        drop_stored_reasoning(&mut request);
    }
    readable_media(request)
}

/// xAI reads JPEG and PNG images and files, inline or from public https links. Rig sends any
/// image, labels an untyped one PNG, and fails the call on audio, video and file ids. So each
/// image leaves typed as the JPEG or PNG it is, media xAI cannot read becomes a note, and a link
/// xAI cannot fetch fails the call.
fn readable_media(mut request: CompletionRequest) -> Result<CompletionRequest, CompletionError> {
    let mut omitted = 0usize;
    for message in request.chat_history.iter_mut() {
        let RigMessage::User { content } = message else {
            continue;
        };
        for part in content.iter_mut() {
            let note = match part {
                UserContent::Image(image) => unreadable_image_note(image)?,
                UserContent::Document(document) => unreadable_document_note(document)?,
                UserContent::Audio(audio) => Some(user_media::note(
                    Kind::Audio,
                    user_media::mime_of(audio.media_type.as_ref()),
                    &Source::of(&audio.data),
                )),
                UserContent::Video(video) => Some(user_media::note(
                    Kind::Video,
                    user_media::mime_of(video.media_type.as_ref()),
                    &Source::of(&video.data),
                )),
                UserContent::Text(_) | UserContent::ToolResult(_) => None,
            };
            if let Some(note) = note {
                *part = UserContent::text(note);
                omitted += 1;
            }
        }
    }
    if omitted > 0 {
        tracing::warn!(omitted, "Replaced attachments xAI cannot read with notes");
    }
    Ok(request)
}

/// Types an image as the JPEG or PNG its bytes hold, or returns the note that replaces it.
fn unreadable_image_note(image: &mut Image) -> Result<Option<String>, CompletionError> {
    user_media::type_image_bytes(image);
    let source = Source::of(&image.data);
    let mime = user_media::image_mime(image, &source);
    let readable = match &source {
        _ if mime.is_some_and(|mime| !matches!(mime, "image/jpeg" | "image/png")) => false,
        Source::Link(_) => true,
        Source::LocalLink(link) => {
            return Err(user_media::unreachable(NAME, Kind::Image, mime, link));
        }
        Source::Inline { .. } => mime.is_some(),
        Source::Text(_) | Source::FileId | Source::Other => false,
    };
    Ok((!readable).then(|| user_media::note(Kind::Image, mime, &source)))
}

/// xAI fetches files from public https links; Rig cannot send it a file id.
fn unreadable_document_note(document: &Document) -> Result<Option<String>, CompletionError> {
    let source = Source::of(&document.data);
    let mime = user_media::mime_of(document.media_type.as_ref());
    match &source {
        Source::LocalLink(link) => Err(user_media::unreachable(NAME, Kind::Document, mime, link)),
        Source::FileId => Ok(Some(user_media::note(Kind::Document, mime, &source))),
        _ => Ok(None),
    }
}

pub struct XAIModel {
    client: rig::providers::xai::Client<AuthorizedHttpClient>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl XAIModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let xai_config = random_provider(&config.xai_config)?;
        let api_key = xai_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::xai::Client::builder()
            .api_key(&api_key)
            .http_client(AuthorizedHttpClient::default());

        if let Some(endpoint) = xai_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(XAIModel {
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

        let mut builder = rig::providers::xai::Client::builder()
            .api_key(api_key)
            .http_client(AuthorizedHttpClient::for_egress(egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(XAIModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for XAIModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for XAIModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(
            self.client.clone(),
            xai_constraints,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn additional_params(&self, history: &Option<History>) -> Option<serde_json::Value> {
        let history = history.as_ref()?;
        let base = history.build_additional_params().ok().flatten();
        let model_name = self
            .default_model
            .as_deref()
            .or(Some(history.model.as_str()));

        if !supports_reasoning_effort(model_name) {
            return base;
        }

        let reasoning = history
            .thinking
            .and_then(|thinking| thinking.xai_reasoning_effort())
            .map(|effort| {
                json!({
                    "reasoning_effort": effort,
                })
            });

        merge_additional_params(base, reasoning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Content, ContentType, HistoryMessage, ImageUrl, MessageContent, Role};
    use crate::llm::test_support::{json_response, responses_completion, serve_once};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use rig::OneOrMany;
    use rig::message::{AssistantContent, DocumentSourceKind, ImageMediaType, Reasoning};
    use std::collections::HashMap;

    const PNG: &str = "iVBORw0KGgo=";
    const WEBP: &str = "UklGRgAAAABXRUJQ";
    const JPEG: [u8; 12] = [
        0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1,
    ];

    fn signed_link(name: &str) -> String {
        format!("https://bucket.s3.amazonaws.com/runs/r/files/0/{name}?X-Amz-Signature=secret")
    }

    fn image(url: &str, media_type: Option<&str>) -> Content {
        Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: url.to_string(),
                detail: None,
                media_type: media_type.map(str::to_string),
                additional_params: None,
            },
        }
    }

    async fn model(endpoint: &str) -> XAIModel {
        XAIModel::from_provider(
            &ModelProvider {
                provider_name: "xai".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(HashMap::from([
                    ("api_key".to_string(), json!("test-key")),
                    ("model_id".to_string(), json!("grok-4")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap()
    }

    fn user_parts_of(body: &Value) -> Vec<Value> {
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["role"] == "user")
            .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
            .collect()
    }

    fn values_of<'a>(parts: &'a [Value], kind: &str, field: &str) -> Vec<&'a Value> {
        parts
            .iter()
            .filter(|part| part["type"] == kind)
            .map(|part| &part[field])
            .collect()
    }

    #[tokio::test]
    async fn images_reach_xai_as_jpeg_or_png_or_as_a_note() {
        let (endpoint, server) = serve_once(json_response(responses_completion("grok-4"))).await;
        let jpeg = STANDARD.encode(JPEG);
        let mut history = History::new(
            "grok-4".to_string(),
            vec![HistoryMessage {
                role: Role::User,
                content: MessageContent::Contents(vec![
                    Content::Text {
                        content_type: ContentType::Text,
                        text: "Describe these.".to_string(),
                    },
                    image(&format!("data:image/png;base64,{PNG}"), Some("image/png")),
                    image(
                        &format!("data:application/octet-stream;base64,{jpeg}"),
                        None,
                    ),
                    image(
                        &format!("data:image/webp;base64,{WEBP}"),
                        Some("image/webp"),
                    ),
                    image(&signed_link("photo.png"), Some("image/png")),
                    image(&signed_link("animation.gif"), None),
                    Content::Document {
                        content_type: ContentType::DocumentUrl,
                        document_url: signed_link("report.pdf"),
                        media_type: Some("application/pdf".to_string()),
                        additional_params: None,
                    },
                ]),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                annotations: None,
            }],
        );
        history.stream = Some(false);

        model(&endpoint).await.invoke(&history, None).await.unwrap();

        let body = server.await.unwrap();
        assert!(!body.contains("animation.gif"), "{body}");
        let body: Value = serde_json::from_str(&body).unwrap();
        let parts = user_parts_of(&body);
        assert_eq!(
            values_of(&parts, "input_image", "image_url"),
            [
                &json!(format!("data:image/png;base64,{PNG}")),
                &json!(format!("data:image/jpeg;base64,{jpeg}")),
                &json!(signed_link("photo.png")),
            ]
        );
        let texts = values_of(&parts, "input_text", "text");
        for note in [
            "[attachment omitted: unsupported file type image/webp]",
            "[attachment omitted: unsupported file type image/gif]",
        ] {
            assert!(texts.contains(&&json!(note)), "{note} in {texts:?}");
        }
        assert_eq!(
            values_of(&parts, "input_file", "file_url"),
            [&json!(signed_link("report.pdf"))]
        );
        assert_eq!(body["store"], false, "{body}");
    }

    fn request(content: Vec<UserContent>) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(vec![
                RigMessage::User {
                    content: OneOrMany::many(content).unwrap(),
                },
                RigMessage::Assistant {
                    id: None,
                    content: OneOrMany::many(vec![
                        AssistantContent::Reasoning(
                            Reasoning::summaries(Vec::new()).with_id("rs_stored".to_string()),
                        ),
                        AssistantContent::text("Looking."),
                    ])
                    .unwrap(),
                },
            ])
            .unwrap(),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    fn replays_reasoning(request: &CompletionRequest) -> bool {
        request.chat_history.iter().any(|message| {
            matches!(message, RigMessage::Assistant { content, .. }
                if content.iter().any(|part| matches!(part, AssistantContent::Reasoning(_))))
        })
    }

    #[test]
    fn only_requests_with_images_or_links_leave_storage() {
        let text_only = xai_constraints(
            "grok-4",
            request(vec![
                UserContent::text("Summarise."),
                UserContent::document("a,b\n1,2\n", Some(rig::message::DocumentMediaType::CSV)),
            ]),
        )
        .unwrap();
        assert_eq!(text_only.additional_params, None);
        assert!(replays_reasoning(&text_only));

        for media in [
            UserContent::image_base64(PNG, Some(ImageMediaType::PNG), None),
            UserContent::document_url(signed_link("report.pdf"), None),
        ] {
            let unstored = xai_constraints("grok-4", request(vec![media.clone()])).unwrap();
            assert_eq!(
                unstored.additional_params,
                Some(json!({"store": false})),
                "{media:?}"
            );
            assert!(!replays_reasoning(&unstored), "{media:?}");
        }
    }

    #[test]
    fn untyped_images_are_typed_by_their_bytes_or_named_in_a_note() {
        let base64 = |data: &str| Image {
            data: DocumentSourceKind::Base64(data.to_string()),
            media_type: None,
            detail: None,
            additional_params: None,
        };

        let mut png = base64(PNG);
        assert_eq!(unreadable_image_note(&mut png).unwrap(), None);
        assert_eq!(png.media_type, Some(ImageMediaType::PNG));

        let mut raw_jpeg = Image {
            data: DocumentSourceKind::Raw(JPEG.to_vec()),
            ..base64("")
        };
        assert_eq!(unreadable_image_note(&mut raw_jpeg).unwrap(), None);
        assert_eq!(raw_jpeg.media_type, Some(ImageMediaType::JPEG));
        assert_eq!(
            raw_jpeg.data,
            DocumentSourceKind::Base64(STANDARD.encode(JPEG))
        );

        for (mut image, note) in [
            (
                base64("R0lGODlhAQABAAAAACw="),
                "[attachment omitted: unsupported file type image/gif]",
            ),
            (
                base64("AAAAAAAAAAAAAAAA"),
                "[attachment omitted: unsupported file type image]",
            ),
            (
                Image {
                    data: DocumentSourceKind::Url(signed_link("scan.BMP")),
                    ..base64("")
                },
                "[attachment omitted: unsupported file type image/bmp]",
            ),
        ] {
            assert_eq!(
                unreadable_image_note(&mut image).unwrap().as_deref(),
                Some(note)
            );
        }

        let mut unnamed_link = Image {
            data: DocumentSourceKind::Url(signed_link("photo")),
            ..base64("")
        };
        assert_eq!(unreadable_image_note(&mut unnamed_link).unwrap(), None);
        assert_eq!(unnamed_link.media_type, None);
    }

    #[test]
    fn links_xai_cannot_fetch_fail_and_media_it_cannot_read_become_notes() {
        for (part, host) in [
            (
                UserContent::image_url(
                    "asset://localhost/%2FUsers%2Fme%2Fscan.png",
                    Some(ImageMediaType::PNG),
                    None,
                ),
                "localhost",
            ),
            (
                UserContent::document_url(
                    "http://bucket.s3.amazonaws.com/runs/r/files/0/report.pdf?X-Amz-Signature=secret",
                    Some(rig::message::DocumentMediaType::PDF),
                ),
                "bucket.s3.amazonaws.com",
            ),
        ] {
            let error = xai_constraints("grok-4", request(vec![part]))
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("from host '{host}'")), "{error}");
            assert!(error.contains("cannot be sent to xAI as a link"), "{error}");
            for secret in ["scan.png", "report.pdf", "X-Amz"] {
                assert!(!error.contains(secret), "{error}");
            }
        }

        let fixed = xai_constraints(
            "grok-4",
            request(vec![
                UserContent::audio("UklGRg==", Some(rig::message::AudioMediaType::WAV)),
                UserContent::Video(rig::message::Video {
                    data: DocumentSourceKind::Url(signed_link("clip.mp4")),
                    media_type: Some(rig::message::VideoMediaType::MP4),
                    additional_params: None,
                }),
                UserContent::Document(Document {
                    data: DocumentSourceKind::FileId("file-abc".to_string()),
                    media_type: None,
                    additional_params: None,
                }),
            ]),
        )
        .unwrap();
        let RigMessage::User { content } = fixed.chat_history.first() else {
            panic!("the user message comes first");
        };
        let notes: Vec<_> = content
            .iter()
            .map(|part| match part {
                UserContent::Text(text) => text.text.as_str(),
                part => panic!("{part:?} must become a note"),
            })
            .collect();
        assert_eq!(
            notes,
            [
                "[attachment omitted: unsupported file type audio/wav]",
                "[attachment omitted: unsupported file type video/mp4]",
                "[attachment omitted: unsupported file type provider file id]",
            ]
        );
    }
}
