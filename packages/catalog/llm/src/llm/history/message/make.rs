/// # Make Message Node
/// Create a new Message object with text or multimodal Message Content
/// Set the message type via Role input.
/// In case of a Tool Message, the associated Tool Call Id has to be provided as well
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
pub struct MakeHistoryMessageNode {}

impl MakeHistoryMessageNode {
    pub fn new() -> Self {
        MakeHistoryMessageNode {}
    }

    fn add_role_pin(node: &mut Node) {
        node.add_input_pin("role", "Role", "Author role", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "Assistant".to_string(),
                        "System".to_string(),
                        "User".to_string(),
                        "Tool".to_string(),
                        "Function".to_string(),
                    ])
                    .build(),
            )
            .set_default_value(Some(json!("User")));
    }

    fn add_type_pin(node: &mut Node) {
        node.add_input_pin("type", "Type", "Message content type", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(CONTENT_TYPES.iter().map(ToString::to_string).collect())
                    .build(),
            )
            .set_default_value(Some(json!("Text")));
    }

    fn content_pin_name(message_type: &str) -> &'static str {
        match message_type {
            "Image" => "image",
            "Audio" => "audio",
            "Video" => "video",
            "Document" => "document",
            _ => "text",
        }
    }

    fn allowed_content_types(role: &str) -> &'static [&'static str] {
        match role {
            "System" => &["Text"],
            "Assistant" | "Tool" | "Function" => &["Text", "Image"],
            _ => CONTENT_TYPES,
        }
    }

    fn sync_type_options(node: &mut Node, role: &str, selected_type: &str) -> String {
        let allowed = Self::allowed_content_types(role);
        let selected_type = if allowed.contains(&selected_type) {
            selected_type
        } else {
            "Text"
        };
        if let Some(type_pin) = node.get_pin_mut_by_name("type") {
            type_pin.set_options(
                PinOptions::new()
                    .set_valid_values(allowed.iter().map(ToString::to_string).collect())
                    .build(),
            );
            type_pin.set_default_value(Some(json!(selected_type)));
        }
        selected_type.to_string()
    }

    fn allowed_mime_types(message_type: &str) -> Option<&'static [&'static str]> {
        match message_type {
            "Image" => Some(IMAGE_MIME_TYPES),
            "Audio" => Some(AUDIO_MIME_TYPES),
            "Video" => Some(VIDEO_MIME_TYPES),
            "Document" => Some(DOCUMENT_MIME_TYPES),
            _ => None,
        }
    }

    fn mime_options(message_type: &str) -> Option<(&'static [&'static str], &'static str)> {
        Self::allowed_mime_types(message_type).map(|types| (types, AUTO_MIME_TYPE))
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

    fn ensure_value_pin(node: &mut Node, message_type: &str) {
        let pin_name = Self::content_pin_name(message_type);
        if node.get_pin_by_name(pin_name).is_none() {
            let friendly_name = match message_type {
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

    fn ensure_mime_pin(node: &mut Node, message_type: &str) {
        let Some((valid_values, default_value)) = Self::mime_options(message_type) else {
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

    fn sync_content_pins(node: &mut Node, message_type: &str) {
        let message_type = if CONTENT_TYPES.contains(&message_type) {
            message_type
        } else {
            "Text"
        };
        let value_pin = Self::content_pin_name(message_type);
        let wants_detail = message_type == "Image";
        let wants_mime = message_type != "Text";

        for pin_name in DYNAMIC_CONTENT_PINS {
            let keep = *pin_name == value_pin
                || (*pin_name == "detail" && wants_detail)
                || (*pin_name == "mime" && wants_mime);
            if !keep {
                Self::remove_pin(node, pin_name);
            }
        }

        Self::ensure_value_pin(node, message_type);
        if wants_detail {
            Self::ensure_detail_pin(node);
        }
        Self::ensure_mime_pin(node, message_type);
    }

    fn ensure_tool_call_id_pin(node: &mut Node) {
        if node.get_pin_by_name("tool_call_id").is_none() {
            node.add_input_pin(
                "tool_call_id",
                "Tool Call Id",
                "Tool Call Identifier",
                VariableType::String,
            )
            .set_default_value(Some(json!("")));
        }
    }

    fn sync_tool_call_id_pin(node: &mut Node, role: &str) {
        if matches!(role, "Tool" | "Function") {
            Self::ensure_tool_call_id_pin(node);
        } else {
            Self::remove_pin(node, "tool_call_id");
        }
    }

    fn seed_signature_pins(node: &mut Node) {
        for message_type in CONTENT_TYPES {
            Self::ensure_value_pin(node, message_type);
        }
        Self::ensure_detail_pin(node);
        Self::ensure_signature_mime_pin(node);
        Self::ensure_tool_call_id_pin(node);
    }

    fn canonical_mime(message_type: &str, mime: &str) -> Option<&'static str> {
        let normalized = mime.trim().to_ascii_lowercase();
        let normalized = match (message_type, normalized.as_str()) {
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

        Self::allowed_mime_types(message_type)?
            .iter()
            .copied()
            .find(|allowed| allowed.eq_ignore_ascii_case(normalized))
    }

    fn selected_mime(message_type: &str, mime: &str) -> flow_like_types::Result<Option<String>> {
        let mime = mime.trim();
        if mime.is_empty() || mime.eq_ignore_ascii_case(AUTO_MIME_TYPE) {
            return Ok(None);
        }

        let canonical = Self::canonical_mime(message_type, mime).ok_or_else(|| {
            anyhow!("MIME type {mime} is not supported for {message_type} content")
        })?;
        Ok(Some(canonical.to_string()))
    }

    fn normalize_media_input(
        value: String,
        message_type: &str,
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
            let canonical = Self::canonical_mime(message_type, declared_mime).ok_or_else(|| {
                anyhow!(
                    "Data URI MIME type {declared_mime} is not supported for {message_type} content"
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

    fn parse_role(role: &str) -> Role {
        match role {
            "Assistant" => Role::Assistant,
            "System" => Role::System,
            "Tool" => Role::Tool,
            "Function" => Role::Function,
            _ => Role::User,
        }
    }

    async fn read_tool_call_id(
        role: &Role,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<Option<String>> {
        if matches!(role, Role::Tool | Role::Function) {
            let tool_call_id: String = context.evaluate_pin("tool_call_id").await?;
            if tool_call_id.trim().is_empty() {
                return Err(anyhow!(
                    "Tool and Function messages require a non-empty Tool Call Id"
                ));
            }
            Ok(Some(tool_call_id))
        } else {
            Ok(None)
        }
    }

    async fn build_content(
        message_type: &str,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<MessageContent> {
        match message_type {
            "Image" => {
                let image: String = context.evaluate_pin("image").await?;
                let detail: String = context.evaluate_pin("detail").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Image", &mime)?;
                Ok(MessageContent::Contents(vec![Content::Image {
                    content_type: ContentType::ImageUrl,
                    image_url: ImageUrl {
                        url: Self::normalize_media_input(image, "Image", media_type.as_deref())?,
                        detail: Some(detail),
                        media_type,
                        additional_params: None,
                    },
                }]))
            }
            "Audio" => {
                let audio: String = context.evaluate_pin("audio").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Audio", &mime)?;
                Ok(MessageContent::Contents(vec![Content::Audio {
                    content_type: ContentType::AudioUrl,
                    audio_url: Self::normalize_media_input(audio, "Audio", media_type.as_deref())?,
                    media_type,
                    additional_params: None,
                }]))
            }
            "Video" => {
                let video: String = context.evaluate_pin("video").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Video", &mime)?;
                Ok(MessageContent::Contents(vec![Content::Video {
                    content_type: ContentType::VideoUrl,
                    video_url: Self::normalize_media_input(video, "Video", media_type.as_deref())?,
                    media_type,
                    additional_params: None,
                }]))
            }
            "Document" => {
                let document: String = context.evaluate_pin("document").await?;
                let mime: String = context.evaluate_pin("mime").await?;
                let media_type = Self::selected_mime("Document", &mime)?;
                Ok(MessageContent::Contents(vec![Content::Document {
                    content_type: ContentType::DocumentUrl,
                    document_url: Self::normalize_media_input(
                        document,
                        "Document",
                        media_type.as_deref(),
                    )?,
                    media_type,
                    additional_params: None,
                }]))
            }
            _ => {
                let text_pin: String = context.evaluate_pin("text").await?;
                if text_pin.trim().is_empty() {
                    return Err(anyhow!("The selected text payload cannot be empty"));
                }
                Ok(MessageContent::Contents(vec![Content::Text {
                    content_type: ContentType::Text,
                    text: text_pin,
                }]))
            }
        }
    }
}

#[async_trait]
impl NodeLogic for MakeHistoryMessageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_make_history_message",
            "Make Message",
            "Creates a chat message with text, image, audio, video, or document content and optional tool metadata",
            "AI/Generative/History/Message",
        );
        node.add_icon("/flow/icons/message.svg");
        node.set_version(4);
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
        Self::add_role_pin(&mut node);
        Self::add_type_pin(&mut node);
        Self::seed_signature_pins(&mut node);

        node.add_output_pin(
            "message",
            "Message",
            "Newly constructed chat message",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let role_input: String = context.evaluate_pin("role").await?;
        let message_type: String = context.evaluate_pin("type").await?;
        let role = Self::parse_role(&role_input);
        if !Self::allowed_content_types(&role_input).contains(&message_type.as_str()) {
            return Err(anyhow!(
                "Rig does not support {message_type} content for {role_input} messages"
            ));
        }
        let tool_call_id = Self::read_tool_call_id(&role, context).await?;
        let content = Self::build_content(&message_type, context).await?;

        let message = HistoryMessage {
            content,
            role,
            name: None,
            tool_call_id,
            tool_calls: None,
            annotations: None,
        };

        context.set_pin_value("message", json!(message)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let type_pin: String = node
            .get_pin_by_name("type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let role_pin: String = node
            .get_pin_by_name("role")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        Self::sync_tool_call_id_pin(node, &role_pin);
        let type_pin = Self::sync_type_options(node, &role_pin, &type_pin);
        Self::sync_content_pins(node, &type_pin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_type(node: &mut Node, message_type: &str) {
        node.get_pin_mut_by_name("type")
            .expect("type pin")
            .set_default_value(Some(json!(message_type)));
    }

    #[test]
    fn exposes_every_rig_user_content_type() {
        let node = MakeHistoryMessageNode::new().get_node();
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
        assert_eq!(node.version, Some(4));
        for pin_name in [
            "text",
            "image",
            "audio",
            "video",
            "document",
            "detail",
            "mime",
            "tool_call_id",
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
    fn content_pin_switching_is_complete_and_stable() {
        let mut node = MakeHistoryMessageNode::new().get_node();

        for (message_type, expected_value_pin, expects_detail) in [
            ("Text", "text", false),
            ("Image", "image", true),
            ("Audio", "audio", false),
            ("Video", "video", false),
            ("Document", "document", false),
        ] {
            set_type(&mut node, message_type);
            MakeHistoryMessageNode::sync_content_pins(&mut node, message_type);

            for pin_name in ["text", "image", "audio", "video", "document"] {
                assert_eq!(
                    node.get_pin_by_name(pin_name).is_some(),
                    pin_name == expected_value_pin,
                    "unexpected value pin for {message_type}: {pin_name}"
                );
            }
            assert_eq!(node.get_pin_by_name("detail").is_some(), expects_detail);
            assert_eq!(
                node.get_pin_by_name("mime").is_some(),
                message_type != "Text"
            );

            let value_pin_id = node
                .get_pin_by_name(expected_value_pin)
                .expect("value pin")
                .id
                .clone();
            let mime_pin_id = node.get_pin_by_name("mime").map(|pin| pin.id.clone());
            MakeHistoryMessageNode::sync_content_pins(&mut node, message_type);
            assert_eq!(
                node.get_pin_by_name(expected_value_pin)
                    .expect("stable value pin")
                    .id,
                value_pin_id
            );
            assert_eq!(
                node.get_pin_by_name("mime").map(|pin| pin.id.clone()),
                mime_pin_id
            );
        }
    }

    #[test]
    fn mime_choices_follow_the_selected_media_type() {
        let mut node = MakeHistoryMessageNode::new().get_node();
        MakeHistoryMessageNode::sync_content_pins(&mut node, "Image");
        let mime_id = node.get_pin_by_name("mime").unwrap().id.clone();

        MakeHistoryMessageNode::sync_content_pins(&mut node, "Audio");
        let mime = node.get_pin_by_name("mime").expect("audio MIME pin");
        assert_eq!(mime.id, mime_id, "compatible MIME pin should be reused");
        assert_eq!(
            mime.options
                .as_ref()
                .unwrap()
                .valid_values
                .as_ref()
                .unwrap(),
            &std::iter::once(AUTO_MIME_TYPE.to_string())
                .chain(AUDIO_MIME_TYPES.iter().map(ToString::to_string))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            MakeHistoryMessageNode::current_string_default(&node, "mime").as_deref(),
            Some(AUTO_MIME_TYPE)
        );
    }

    #[test]
    fn role_options_match_rig_message_capabilities() {
        let mut node = MakeHistoryMessageNode::new().get_node();
        assert!(
            node.get_pin_by_name("role")
                .and_then(|pin| pin.options.as_ref())
                .and_then(|options| options.valid_values.as_ref())
                .is_some_and(|roles| roles.iter().any(|role| role == "Function"))
        );
        let selected = MakeHistoryMessageNode::sync_type_options(&mut node, "Assistant", "Audio");
        assert_eq!(selected, "Text");
        assert_eq!(
            node.get_pin_by_name("type")
                .and_then(|pin| pin.options.as_ref())
                .and_then(|options| options.valid_values.as_ref())
                .expect("assistant content choices"),
            &vec!["Text".to_string(), "Image".to_string()]
        );

        let selected = MakeHistoryMessageNode::sync_type_options(&mut node, "System", "Image");
        assert_eq!(selected, "Text");
        assert_eq!(
            node.get_pin_by_name("type")
                .and_then(|pin| pin.options.as_ref())
                .and_then(|options| options.valid_values.as_ref())
                .expect("system content choices"),
            &vec!["Text".to_string()]
        );

        let selected = MakeHistoryMessageNode::sync_type_options(&mut node, "Function", "Image");
        assert_eq!(selected, "Image");
        MakeHistoryMessageNode::sync_tool_call_id_pin(&mut node, "Function");
        assert!(node.get_pin_by_name("tool_call_id").is_some());
        assert!(
            node.get_pin_by_name("tool_call_id")
                .is_some_and(|pin| pin.default_value.is_some())
        );
        assert_eq!(
            MakeHistoryMessageNode::parse_role("Function"),
            Role::Function
        );
        MakeHistoryMessageNode::sync_tool_call_id_pin(&mut node, "User");
        assert!(node.get_pin_by_name("tool_call_id").is_none());
    }

    #[test]
    fn explicit_mime_wraps_bare_media_and_inference_sources_are_preserved() {
        assert_eq!(
            MakeHistoryMessageNode::normalize_media_input(
                "YWJj".into(),
                "Audio",
                Some("audio/mp3")
            )
            .unwrap(),
            "data:audio/mp3;base64,YWJj"
        );
        assert_eq!(
            MakeHistoryMessageNode::normalize_media_input(
                "https://example.com/audio.mp3".into(),
                "Audio",
                None
            )
            .unwrap(),
            "https://example.com/audio.mp3"
        );
        assert_eq!(
            MakeHistoryMessageNode::normalize_media_input(
                "data:audio/wav;base64,YWJj".into(),
                "Audio",
                None
            )
            .unwrap(),
            "data:audio/wav;base64,YWJj"
        );
        assert_eq!(
            MakeHistoryMessageNode::normalize_media_input(
                "asset://localhost/audio.mp3".into(),
                "Audio",
                None
            )
            .unwrap(),
            "asset://localhost/audio.mp3"
        );
        assert_eq!(
            MakeHistoryMessageNode::normalize_media_input("file_id:file-123".into(), "Audio", None)
                .unwrap(),
            "file_id:file-123"
        );
    }

    #[test]
    fn auto_mime_is_unset_and_rejected_for_ambiguous_bare_payloads() {
        assert_eq!(
            MakeHistoryMessageNode::selected_mime("Image", AUTO_MIME_TYPE).unwrap(),
            None
        );
        assert_eq!(
            MakeHistoryMessageNode::selected_mime("Image", "").unwrap(),
            None
        );
        assert_eq!(
            MakeHistoryMessageNode::selected_mime("Image", "image/jpeg").unwrap(),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            MakeHistoryMessageNode::selected_mime("Audio", "audio/mpeg").unwrap(),
            Some("audio/mp3".to_string())
        );
        assert!(MakeHistoryMessageNode::selected_mime("Audio", "image/png").is_err());

        let error = MakeHistoryMessageNode::normalize_media_input("YWJj".into(), "Audio", None)
            .expect_err("bare payload needs a MIME type");
        assert!(error.to_string().contains("MIME type"));
        assert!(
            MakeHistoryMessageNode::normalize_media_input("file_id:".into(), "Audio", None)
                .is_err()
        );
        assert!(
            MakeHistoryMessageNode::normalize_media_input("file_id:   ".into(), "Audio", None)
                .is_err()
        );
        assert!(
            MakeHistoryMessageNode::normalize_media_input("   ".into(), "Audio", None).is_err()
        );
        assert!(
            MakeHistoryMessageNode::normalize_media_input(
                "data:image/png;base64,YWJj".into(),
                "Audio",
                None
            )
            .is_err()
        );
        assert!(
            MakeHistoryMessageNode::normalize_media_input(
                "data:audio/wav;base64,YWJj".into(),
                "Audio",
                Some("audio/mp3")
            )
            .is_err()
        );
    }
}
