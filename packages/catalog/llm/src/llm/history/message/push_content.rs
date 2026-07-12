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
const AUTO_MIME_TYPE: &str = "Auto";
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

    fn allowed_mime_types(content_type: &str) -> Option<&'static [&'static str]> {
        match content_type {
            "Image" => Some(IMAGE_MIME_TYPES),
            "Audio" => Some(AUDIO_MIME_TYPES),
            "Video" => Some(VIDEO_MIME_TYPES),
            "Document" => Some(DOCUMENT_MIME_TYPES),
            _ => None,
        }
    }

    fn mime_options(content_type: &str) -> Option<(&'static [&'static str], &'static str)> {
        Self::allowed_mime_types(content_type).map(|types| (types, AUTO_MIME_TYPE))
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
                format!("{friendly_name} URL, data URI, file_id reference, or bare base64 payload")
            };
            node.add_input_pin(pin_name, friendly_name, &description, VariableType::String)
                .set_default_value(Some(json!("")));
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
                "Auto infers MIME from a URL or data URI; select a type for bare base64",
                VariableType::String,
            );
        }

        let current = Self::current_string_default(node, "mime");
        let mime = node
            .get_pin_mut_by_name("mime")
            .expect("MIME pin was just ensured");
        mime.friendly_name = "MIME Type".to_string();
        mime.description =
            "Auto infers MIME from a URL or data URI; select a type for bare base64".to_string();
        mime.set_options(
            PinOptions::new()
                .set_valid_values(
                    std::iter::once(AUTO_MIME_TYPE.to_string())
                        .chain(valid_values.iter().map(ToString::to_string))
                        .collect(),
                )
                .build(),
        );
        if !current
            .as_deref()
            .is_some_and(|value| value == AUTO_MIME_TYPE || valid_values.contains(&value))
        {
            mime.set_default_value(Some(json!(default_value)));
        }
    }

    fn ensure_signature_mime_pin(node: &mut Node) {
        if node.get_pin_by_name("mime").is_none() {
            node.add_input_pin(
                "mime",
                "MIME Type",
                "Auto infers MIME from a URL or data URI; select a type for bare base64",
                VariableType::String,
            );
        }

        let mime = node
            .get_pin_mut_by_name("mime")
            .expect("MIME pin was just ensured");
        mime.set_options(
            PinOptions::new()
                .set_valid_values(
                    std::iter::once(AUTO_MIME_TYPE)
                        .chain(IMAGE_MIME_TYPES.iter().copied())
                        .chain(AUDIO_MIME_TYPES.iter().copied())
                        .chain(VIDEO_MIME_TYPES.iter().copied())
                        .chain(DOCUMENT_MIME_TYPES.iter().copied())
                        .map(ToString::to_string)
                        .collect(),
                )
                .build(),
        );
        mime.set_default_value(Some(json!(AUTO_MIME_TYPE)));
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

    fn seed_signature_pins(node: &mut Node) {
        for content_type in CONTENT_TYPES {
            Self::ensure_value_pin(node, content_type);
        }
        Self::ensure_detail_pin(node);
        Self::ensure_signature_mime_pin(node);
    }

    fn canonical_mime(content_type: &str, mime: &str) -> Option<&'static str> {
        let normalized = mime.trim().to_ascii_lowercase();
        let normalized = match (content_type, normalized.as_str()) {
            ("Image", "image/jpg") => "image/jpeg",
            ("Audio", "audio/mpeg" | "audio/mpeg3") => "audio/mp3",
            ("Audio", "audio/x-wav" | "audio/wave") => "audio/wav",
            ("Audio", "audio/x-aiff") => "audio/aiff",
            ("Audio", "audio/mp4" | "audio/x-m4a") => "audio/m4a",
            ("Video", "video/x-msvideo") => "video/avi",
            ("Video", "video/quicktime") => "video/mov",
            ("Document", "application/rtf") => "text/rtf",
            ("Document", "application/javascript" | "text/javascript" | "text/x-javascript") => {
                "application/x-javascript"
            }
            ("Document", "text/md" | "text/x-markdown") => "text/markdown",
            ("Document", "text/x-python") => "application/x-python",
            ("Document", "application/xml") => "text/xml",
            _ => normalized.as_str(),
        };

        Self::allowed_mime_types(content_type)?
            .iter()
            .copied()
            .find(|allowed| allowed.eq_ignore_ascii_case(normalized))
    }

    fn selected_mime(content_type: &str, mime: &str) -> flow_like_types::Result<Option<String>> {
        let mime = mime.trim();
        if mime.is_empty() || mime.eq_ignore_ascii_case(AUTO_MIME_TYPE) {
            return Ok(None);
        }

        let canonical = Self::canonical_mime(content_type, mime).ok_or_else(|| {
            anyhow!("MIME type {mime} is not supported for {content_type} content")
        })?;
        Ok(Some(canonical.to_string()))
    }

    fn normalize_media_input(
        value: String,
        content_type: &str,
        mime: Option<&str>,
    ) -> flow_like_types::Result<String> {
        if value.trim().is_empty() {
            return Err(anyhow!("The selected media payload cannot be empty"));
        }

        if let Some(file_id) = value.strip_prefix("file_id:") {
            if file_id.trim().is_empty() {
                return Err(anyhow!("A file_id reference must include an identifier"));
            }
            return Ok(value);
        }

        if let Some(data_uri) = value.strip_prefix("data:") {
            let declared_mime = data_uri
                .split([';', ','])
                .next()
                .map(str::trim)
                .filter(|mime| !mime.is_empty())
                .ok_or_else(|| anyhow!("A data URI must declare its MIME type"))?;
            let canonical = Self::canonical_mime(content_type, declared_mime).ok_or_else(|| {
                anyhow!(
                    "Data URI MIME type {declared_mime} is not supported for {content_type} content"
                )
            })?;
            if let Some(selected) = mime
                && !canonical.eq_ignore_ascii_case(selected)
            {
                return Err(anyhow!(
                    "Data URI MIME type {declared_mime} does not match selected MIME type {selected}"
                ));
            }
            return Ok(value);
        }

        let has_uri_scheme = value.split_once(':').is_some_and(|(scheme, remainder)| {
            let mut chars = scheme.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
                && !remainder.is_empty()
        });

        if has_uri_scheme {
            Ok(value)
        } else {
            let mime = mime.ok_or_else(|| {
                anyhow!("Select a MIME type when providing a bare base64 payload")
            })?;
            Ok(format!("data:{mime};base64,{value}"))
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
        node.set_version(5);
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
        Self::seed_signature_pins(&mut node);

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
                if text.trim().is_empty() {
                    return Err(anyhow!("The selected text payload cannot be empty"));
                }
                content.push(Content::Text {
                    content_type: ContentType::Text,
                    text,
                });
            }
            "Image" => {
                let image: String = context.evaluate_pin("image").await?;
                let detail: String = context.evaluate_pin("detail").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Image", &mime)?;

                content.push(Content::Image {
                    content_type: ContentType::ImageUrl,
                    image_url: ImageUrl {
                        url: Self::normalize_media_input(image, "Image", media_type.as_deref())?,
                        detail: Some(detail),
                        media_type,
                        additional_params: None,
                    },
                });
            }
            "Audio" => {
                let audio: String = context.evaluate_pin("audio").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Audio", &mime)?;
                content.push(Content::Audio {
                    content_type: ContentType::AudioUrl,
                    audio_url: Self::normalize_media_input(audio, "Audio", media_type.as_deref())?,
                    media_type,
                    additional_params: None,
                });
            }
            "Video" => {
                let video: String = context.evaluate_pin("video").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Video", &mime)?;
                content.push(Content::Video {
                    content_type: ContentType::VideoUrl,
                    video_url: Self::normalize_media_input(video, "Video", media_type.as_deref())?,
                    media_type,
                    additional_params: None,
                });
            }
            "Document" => {
                let document: String = context.evaluate_pin("document").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Document", &mime)?;
                content.push(Content::Document {
                    content_type: ContentType::DocumentUrl,
                    document_url: Self::normalize_media_input(
                        document,
                        "Document",
                        media_type.as_deref(),
                    )?,
                    media_type,
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
        assert_eq!(node.version, Some(5));
        for pin_name in [
            "text", "image", "audio", "video", "document", "detail", "mime",
        ] {
            let pin = node
                .get_pin_by_name(pin_name)
                .unwrap_or_else(|| panic!("signature is missing {pin_name}"));
            assert!(
                pin.default_value.is_some(),
                "conditional signature pin {pin_name} must be optional"
            );
        }

        let mime_choices = node
            .get_pin_by_name("mime")
            .and_then(|pin| pin.options.as_ref())
            .and_then(|options| options.valid_values.as_ref())
            .expect("signature MIME choices");
        for mime in [
            AUTO_MIME_TYPE,
            "image/png",
            "audio/mp3",
            "video/mp4",
            "application/pdf",
        ] {
            assert!(mime_choices.iter().any(|choice| choice == mime));
        }
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
                &std::iter::once(AUTO_MIME_TYPE.to_string())
                    .chain(expected.iter().map(ToString::to_string))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                PushContentNode::current_string_default(&node, "mime").as_deref(),
                Some(AUTO_MIME_TYPE)
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
    fn explicit_mime_wraps_bare_media_and_inference_sources_are_preserved() {
        assert_eq!(
            PushContentNode::normalize_media_input(
                "JVBERg==".into(),
                "Document",
                Some("application/pdf")
            )
            .unwrap(),
            "data:application/pdf;base64,JVBERg=="
        );
        assert_eq!(
            PushContentNode::normalize_media_input(
                "http://example.com/video.mp4".into(),
                "Video",
                None
            )
            .unwrap(),
            "http://example.com/video.mp4"
        );
        assert_eq!(
            PushContentNode::normalize_media_input(
                "data:video/webm;base64,AAAA".into(),
                "Video",
                None
            )
            .unwrap(),
            "data:video/webm;base64,AAAA"
        );
        assert_eq!(
            PushContentNode::normalize_media_input("s3://bucket/video.mp4".into(), "Video", None)
                .unwrap(),
            "s3://bucket/video.mp4"
        );
        assert_eq!(
            PushContentNode::normalize_media_input("file_id:file-123".into(), "Video", None)
                .unwrap(),
            "file_id:file-123"
        );
    }

    #[test]
    fn auto_mime_is_unset_and_rejected_for_ambiguous_bare_payloads() {
        assert_eq!(
            PushContentNode::selected_mime("Video", AUTO_MIME_TYPE).unwrap(),
            None
        );
        assert_eq!(PushContentNode::selected_mime("Video", "").unwrap(), None);
        assert_eq!(
            PushContentNode::selected_mime("Video", "video/webm").unwrap(),
            Some("video/webm".to_string())
        );
        assert_eq!(
            PushContentNode::selected_mime("Document", "application/xml").unwrap(),
            Some("text/xml".to_string())
        );
        assert!(PushContentNode::selected_mime("Video", "audio/mp3").is_err());

        let error = PushContentNode::normalize_media_input("AAAA".into(), "Video", None)
            .expect_err("bare payload needs a MIME type");
        assert!(error.to_string().contains("MIME type"));
        assert!(PushContentNode::normalize_media_input("file_id:".into(), "Video", None).is_err());
        assert!(
            PushContentNode::normalize_media_input("file_id:   ".into(), "Video", None).is_err()
        );
        assert!(PushContentNode::normalize_media_input("   ".into(), "Video", None).is_err());
        assert!(
            PushContentNode::normalize_media_input(
                "data:audio/mp3;base64,AAAA".into(),
                "Video",
                None
            )
            .is_err()
        );
        assert!(
            PushContentNode::normalize_media_input(
                "data:video/mp4;base64,AAAA".into(),
                "Video",
                Some("video/webm")
            )
            .is_err()
        );
    }
}
