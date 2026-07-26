use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json, reqwest};

use super::attachment_from_path::mime_from_extension;
use super::{Attachment, ComplexAttachment};

/// Derives a display filename from a URL's final path segment (percent-decoded).
fn file_name_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let segment = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .next_back()?;
    let decoded = urlencoding::decode(segment)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| segment.to_string());
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AttachmentFromUrlNode {}

impl AttachmentFromUrlNode {
    pub fn new() -> Self {
        AttachmentFromUrlNode {}
    }
}

#[async_trait]
impl NodeLogic for AttachmentFromUrlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_attachment_from_signed_url",
            "From Signed URL",
            "Get the URL from an attachment",
            "Events/Chat/Attachments",
        );
        node.add_icon("/flow/icons/paperclip.svg");

        node.add_output_pin(
            "attachment",
            "Attachment",
            "Attachment to the Chat",
            VariableType::Struct,
        )
        .set_schema::<Attachment>();

        node.add_input_pin("signed_url", "Signed URL", "", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let signed_url: String = context.evaluate_pin("signed_url").await?;

        let name = file_name_from_url(&signed_url);
        let content_type = name
            .as_deref()
            .and_then(|name| name.rsplit('.').next())
            .filter(|ext| !ext.is_empty())
            .map(|ext| mime_from_extension(&ext.to_lowercase()).to_string());

        // Populate name/type when cheaply derivable so downstream receivers are
        // not left guessing; fall back to the bare URL form when the URL carries
        // no usable filename.
        let attachment = match (name, content_type) {
            (None, None) => Attachment::Url(signed_url),
            (name, content_type) => Attachment::Complex(ComplexAttachment {
                url: signed_url,
                preview_text: None,
                thumbnail_url: None,
                name,
                size: None,
                r#type: content_type,
                anchor: None,
                page: None,
            }),
        };

        context
            .set_pin_value("attachment", json!(attachment))
            .await?;

        return Ok(());
    }
}
