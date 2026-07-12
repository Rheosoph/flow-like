use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::{
    Content, ContentType, HistoryMessage, ImageUrl, MessageContent, Role,
};
use flow_like_types::{Value, anyhow, async_trait, json::json};

const CONTENT_TYPES: &[&str] = &["Text", "Image", "Audio", "Video", "Document"];
const IMAGE_MIME_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/heic",
    "image/heif",
    "image/svg+xml",
];
const AUDIO_MIME_TYPES: &[&str] = &[
    "audio/wav",
    "audio/mp3",
    "audio/aiff",
    "audio/aac",
    "audio/ogg",
    "audio/flac",
    "audio/m4a",
    "audio/pcm16",
    "audio/pcm24",
];
const VIDEO_MIME_TYPES: &[&str] = &[
    "video/avi",
    "video/mp4",
    "video/mpeg",
    "video/mov",
    "video/webm",
];
const DOCUMENT_MIME_TYPES: &[&str] = &[
    "application/pdf",
    "text/plain",
    "text/rtf",
    "text/html",
    "text/css",
    "text/markdown",
    "text/csv",
    "text/xml",
    "application/x-javascript",
    "application/x-python",
];
const DYNAMIC_CONTENT_PINS: &[&str] = &[
    "text", "image", "audio", "video", "document", "detail", "mime",
];

#[crate::register_node]
#[derive(Default)]
pub struct PushContentNode {}

impl PushContentNode {
    pub fn new() -> Self {
        PushContentNode {}
    }

    fn add_type_pin(node: &mut Node) {
        node.add_input_pin("type", "Type", "Content type", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(CONTENT_TYPES.iter().map(ToString::to_string).collect())
                    .build(),
            )
            .set_default_value(Some(json!("Text")));
    }

    fn content_pin_name(content_type: &str) -> &'static str {
        match content_type {
            "Image" => "image",
            "Audio" => "audio",
            "Video" => "video",
            "Document" => "document",
            _ => "text",
        }
    }

    fn supports_content(role: &Role, content_type: &str) -> bool {
        match role {
            Role::System => content_type == "Text",
            Role::Assistant | Role::Tool | Role::Function => {
                matches!(content_type, "Text" | "Image")
            }
            Role::User => CONTENT_TYPES.contains(&content_type),
        }
    }

    fn mime_options(content_type: &str) -> Option<(&'static [&'static str], &'static str)> {
        match content_type {
            "Image" => Some((IMAGE_MIME_TYPES, "image/png")),
            "Audio" => Some((AUDIO_MIME_TYPES, "audio/mp3")),
            "Video" => Some((VIDEO_MIME_TYPES, "video/mp4")),
            "Document" => Some((DOCUMENT_MIME_TYPES, "application/pdf")),
            _ => None,
        }
    }

    fn current_string_default(node: &Node, pin_name: &str) -> Option<String> {
        node.get_pin_by_name(pin_name)
            .and_then(|pin| pin.default_value.as_deref())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }

    fn remove_pin(node: &mut Node, pin_name: &str) {
        if let Some(pin_id) = node.get_pin_by_name(pin_name).map(|pin| pin.id.clone()) {
            node.pins.remove(&pin_id);
        }
    }

    fn ensure_value_pin(node: &mut Node, content_type: &str) {
        let pin_name = Self::content_pin_name(content_type);
        if node.get_pin_by_name(pin_name).is_none() {
            let friendly_name = match content_type {
                "Image" => "Image",
                "Audio" => "Audio",
                "Video" => "Video",
                "Document" => "Document",
                _ => "Text",
            };
            let description = if pin_name == "text" {
                "Text content".to_string()
            } else {
                format!("{friendly_name} URL, data URI, or bare base64 payload")
            };
            node.add_input_pin(pin_name, friendly_name, &description, VariableType::String);
        }
    }

    fn ensure_detail_pin(node: &mut Node) {
        if node.get_pin_by_name("detail").is_none() {
            node.add_input_pin(
                "detail",
                "Detail",
                "Image resolution detail level",
                VariableType::String,
            );
        }

        let valid_values = ["auto", "low", "high"];
        let current = Self::current_string_default(node, "detail");
        let detail = node
            .get_pin_mut_by_name("detail")
            .expect("detail pin was just ensured");
        detail.set_options(
            PinOptions::new()
                .set_valid_values(valid_values.iter().map(ToString::to_string).collect())
                .build(),
        );
        if !current
            .as_deref()
            .is_some_and(|value| valid_values.contains(&value))
        {
            detail.set_default_value(Some(json!("auto")));
        }
    }

    fn ensure_mime_pin(node: &mut Node, content_type: &str) {
        let Some((valid_values, default_value)) = Self::mime_options(content_type) else {
            Self::remove_pin(node, "mime");
            return;
        };

        if node.get_pin_by_name("mime").is_none() {
            node.add_input_pin(
                "mime",
                "MIME Type",
                "MIME type used when wrapping a bare base64 payload",
                VariableType::String,
            );
        }

        let current = Self::current_string_default(node, "mime");
        let mime = node
            .get_pin_mut_by_name("mime")
            .expect("MIME pin was just ensured");
        mime.friendly_name = "MIME Type".to_string();
        mime.description = "MIME type used when wrapping a bare base64 payload".to_string();
        mime.set_options(
            PinOptions::new()
                .set_valid_values(valid_values.iter().map(ToString::to_string).collect())
                .build(),
        );
        if !current
            .as_deref()
            .is_some_and(|value| valid_values.contains(&value))
        {
            mime.set_default_value(Some(json!(default_value)));
        }
    }

    fn sync_content_pins(node: &mut Node, content_type: &str) {
        let content_type = if CONTENT_TYPES.contains(&content_type) {
            content_type
        } else {
            "Text"
        };
        let value_pin = Self::content_pin_name(content_type);
        let wants_detail = content_type == "Image";
        let wants_mime = content_type != "Text";

        for pin_name in DYNAMIC_CONTENT_PINS {
            let keep = *pin_name == value_pin
                || (*pin_name == "detail" && wants_detail)
                || (*pin_name == "mime" && wants_mime);
            if !keep {
                Self::remove_pin(node, pin_name);
            }
        }

        Self::ensure_value_pin(node, content_type);
        if wants_detail {
            Self::ensure_detail_pin(node);
        }
        Self::ensure_mime_pin(node, content_type);
    }

    fn normalize_media_input(value: String, mime: &str) -> String {
        let has_uri_scheme = value.split_once(':').is_some_and(|(scheme, remainder)| {
            let mut chars = scheme.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
                && !remainder.is_empty()
        });

        if has_uri_scheme {
            value
        } else {
            format!("data:{mime};base64,{value}")
        }
    }

    fn normalized_content(message: &HistoryMessage) -> Vec<Content> {
        match &message.content {
            MessageContent::String(text) => vec![Content::Text {
                content_type: ContentType::Text,
                text: text.clone(),
            }],
            MessageContent::Contents(contents) => contents.clone(),
        }
    }
}

#[async_trait]
impl NodeLogic for PushContentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_push_content",
            "Push Content",
            "Appends text, image, audio, video, or document parts onto a chat message",
            "AI/Generative/History/Message",
        );
        node.set_version(3);
        node.add_icon("/flow/icons/message.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger when ready to append content",
            VariableType::Execution,
        );

        node.add_input_pin(
            "message",
            "Message",
            "Message to extend",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        Self::add_type_pin(&mut node);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion once content is appended",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message_out",
            "Message",
            "Updated message with additional content",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut message: HistoryMessage = context.evaluate_pin("message").await?;
        let content_type: String = context.evaluate_pin("type").await?;
        if !Self::supports_content(&message.role, &content_type) {
            return Err(anyhow!(
                "Rig does not support {content_type} content for {:?} messages",
                message.role
            ));
        }
        let mut content = Self::normalized_content(&message);

        match content_type.as_str() {
            "Text" => {
                let text: String = context.evaluate_pin("text").await?;
                content.push(Content::Text {
                    content_type: ContentType::Text,
                    text,
                });
            }
            "Image" => {
                let image: String = context.evaluate_pin("image").await?;
                let detail: String = context.evaluate_pin("detail").await?;
                let mime: String = context.evaluate_pin("mime").await?;

                content.push(Content::Image {
                    content_type: ContentType::ImageUrl,
                    image_url: ImageUrl {
                        url: Self::normalize_media_input(image, &mime),
                        detail: Some(detail),
                        media_type: Some(mime),
                        additional_params: None,
                    },
                });
            }
            "Audio" => {
                let audio: String = context.evaluate_pin("audio").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                content.push(Content::Audio {
                    content_type: ContentType::AudioUrl,
                    audio_url: Self::normalize_media_input(audio, &mime),
                    media_type: Some(mime),
                    additional_params: None,
                });
            }
            "Video" => {
                let video: String = context.evaluate_pin("video").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                content.push(Content::Video {
                    content_type: ContentType::VideoUrl,
                    video_url: Self::normalize_media_input(video, &mime),
                    media_type: Some(mime),
                    additional_params: None,
                });
            }
            "Document" => {
                let document: String = context.evaluate_pin("document").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                content.push(Content::Document {
                    content_type: ContentType::DocumentUrl,
                    document_url: Self::normalize_media_input(document, &mime),
                    media_type: Some(mime),
                    additional_params: None,
                });
            }
            _ => {}
        }

        message.content = MessageContent::Contents(content);

        context.set_pin_value("message_out", json!(message)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let type_pin: String = node
            .get_pin_by_name("type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        Self::sync_content_pins(node, &type_pin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_every_rig_user_content_type() {
        let node = PushContentNode::new().get_node();
        let types = node
            .get_pin_by_name("type")
            .and_then(|pin| pin.options.as_ref())
            .and_then(|options| options.valid_values.as_ref())
            .expect("content type choices");

        assert_eq!(
            types,
            &CONTENT_TYPES
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(node.version, Some(3));
    }

    #[test]
    fn content_pin_switching_removes_stale_media_pins() {
        let mut node = PushContentNode::new().get_node();

        PushContentNode::sync_content_pins(&mut node, "Image");
        assert!(node.get_pin_by_name("image").is_some());
        assert!(node.get_pin_by_name("detail").is_some());
        assert!(node.get_pin_by_name("mime").is_some());

        PushContentNode::sync_content_pins(&mut node, "Document");
        assert!(node.get_pin_by_name("image").is_none());
        assert!(node.get_pin_by_name("detail").is_none());
        assert!(node.get_pin_by_name("document").is_some());
        assert!(node.get_pin_by_name("mime").is_some());

        let document_id = node.get_pin_by_name("document").unwrap().id.clone();
        let mime_id = node.get_pin_by_name("mime").unwrap().id.clone();
        PushContentNode::sync_content_pins(&mut node, "Document");
        assert_eq!(node.get_pin_by_name("document").unwrap().id, document_id);
        assert_eq!(node.get_pin_by_name("mime").unwrap().id, mime_id);

        PushContentNode::sync_content_pins(&mut node, "Text");
        assert!(node.get_pin_by_name("document").is_none());
        assert!(node.get_pin_by_name("mime").is_none());
        assert!(node.get_pin_by_name("text").is_some());
    }

    #[test]
    fn each_media_type_has_rig_supported_mime_choices() {
        let mut node = PushContentNode::new().get_node();
        for (content_type, expected) in [
            ("Image", IMAGE_MIME_TYPES),
            ("Audio", AUDIO_MIME_TYPES),
            ("Video", VIDEO_MIME_TYPES),
            ("Document", DOCUMENT_MIME_TYPES),
        ] {
            PushContentNode::sync_content_pins(&mut node, content_type);
            let actual = node
                .get_pin_by_name("mime")
                .and_then(|pin| pin.options.as_ref())
                .and_then(|options| options.valid_values.as_ref())
                .expect("MIME choices");
            assert_eq!(
                actual,
                &expected.iter().map(ToString::to_string).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn message_roles_reject_media_rig_cannot_represent() {
        assert!(PushContentNode::supports_content(&Role::User, "Audio"));
        assert!(PushContentNode::supports_content(&Role::Assistant, "Image"));
        assert!(!PushContentNode::supports_content(
            &Role::Assistant,
            "Audio"
        ));
        assert!(!PushContentNode::supports_content(&Role::Tool, "Video"));
        assert!(!PushContentNode::supports_content(&Role::System, "Image"));
    }

    #[test]
    fn bare_media_is_wrapped_but_urls_and_data_uris_are_preserved() {
        assert_eq!(
            PushContentNode::normalize_media_input("JVBERg==".into(), "application/pdf"),
            "data:application/pdf;base64,JVBERg=="
        );
        assert_eq!(
            PushContentNode::normalize_media_input(
                "http://example.com/video.mp4".into(),
                "video/mp4"
            ),
            "http://example.com/video.mp4"
        );
        assert_eq!(
            PushContentNode::normalize_media_input(
                "data:video/webm;base64,AAAA".into(),
                "video/mp4"
            ),
            "data:video/webm;base64,AAAA"
        );
        assert_eq!(
            PushContentNode::normalize_media_input("s3://bucket/video.mp4".into(), "video/mp4"),
            "s3://bucket/video.mp4"
        );
    }
}
