use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::time::Duration;

use super::{Attachment, ComplexAttachment};

/// Default signed-URL lifetime. Attachments are frozen into a run's payload and
/// re-read for the run's lifetime, so the URL must outlive the 24h run/executor
/// TTL rather than the object store's short 1h default.
const DEFAULT_SIGNED_URL_EXPIRATION_SECS: i64 = 24 * 60 * 60;

pub(crate) fn mime_from_extension(extension: &str) -> &'static str {
    match extension {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "svg" => "image/svg+xml",
        "avi" => "video/avi",
        "mp4" => "video/mp4",
        "mpeg" | "mpg" => "video/mpeg",
        "mov" => "video/mov",
        "webm" => "video/webm",
        "wav" | "wave" => "audio/wav",
        "mp3" => "audio/mpeg",
        "aif" | "aiff" => "audio/aiff",
        "aac" => "audio/aac",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/m4a",
        "pcm16" => "audio/pcm16",
        "pcm24" => "audio/pcm24",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "application/x-javascript",
        "py" => "application/x-python",
        "txt" => "text/plain",
        "rtf" => "text/rtf",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AttachmentFromPathNode {}

impl AttachmentFromPathNode {
    pub fn new() -> Self {
        AttachmentFromPathNode {}
    }
}

#[async_trait]
impl NodeLogic for AttachmentFromPathNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_attachment_from_path",
            "From Path",
            "Creates an attachment from a FlowPath with optional metadata",
            "Events/Chat/Attachments",
        );
        node.add_icon("/flow/icons/paperclip.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "path",
            "Path",
            "FlowPath to create attachment from",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "name",
            "Name",
            "Display name for the attachment (optional, defaults to filename)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "preview_text",
            "Preview Text",
            "Preview text/description for the attachment (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "page",
            "Page",
            "Page number reference (optional, for documents)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_input_pin(
            "anchor",
            "Anchor",
            "Anchor/section reference within the document (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "expiration",
            "Expiration (seconds)",
            "Expiration time for the signed URL",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_SIGNED_URL_EXPIRATION_SECS)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "attachment",
            "Attachment",
            "The created attachment",
            VariableType::Struct,
        )
        .set_schema::<Attachment>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let flow_path: FlowPath = context.evaluate_pin("path").await?;
        let name: String = context.evaluate_pin("name").await?;
        let preview_text: String = context.evaluate_pin("preview_text").await?;
        let page: i64 = context.evaluate_pin("page").await?;
        let anchor: String = context.evaluate_pin("anchor").await?;
        let expiration: i64 = context.evaluate_pin("expiration").await?;

        let runtime_path = flow_path.to_runtime(context).await?;

        let signed_url = runtime_path
            .store
            .sign(
                "GET",
                &runtime_path.path,
                Duration::from_secs(expiration as u64),
            )
            .await?;

        let filename = runtime_path.path.filename().map(|s| s.to_string());

        let extension = runtime_path.path.extension().map(|s| s.to_lowercase());

        let content_type = extension
            .as_deref()
            .map(mime_from_extension)
            .map(ToString::to_string);

        let display_name = if name.is_empty() {
            filename.clone()
        } else {
            Some(name)
        };

        let attachment = ComplexAttachment {
            url: signed_url.to_string(),
            preview_text: if preview_text.is_empty() {
                None
            } else {
                Some(preview_text)
            },
            thumbnail_url: None,
            name: display_name,
            size: None,
            r#type: content_type,
            anchor: if anchor.is_empty() {
                None
            } else {
                Some(anchor)
            },
            page: if page < 0 { None } else { Some(page as u32) },
        };

        context
            .set_pin_value("attachment", json!(Attachment::Complex(attachment)))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::mime_from_extension;

    #[test]
    fn maps_every_rig_native_media_family_from_paths() {
        assert_eq!(mime_from_extension("heic"), "image/heic");
        assert_eq!(mime_from_extension("heif"), "image/heif");
        assert_eq!(mime_from_extension("aiff"), "audio/aiff");
        assert_eq!(mime_from_extension("flac"), "audio/flac");
        assert_eq!(mime_from_extension("m4a"), "audio/m4a");
        assert_eq!(mime_from_extension("avi"), "video/avi");
        assert_eq!(mime_from_extension("mov"), "video/mov");
        assert_eq!(mime_from_extension("rtf"), "text/rtf");
        assert_eq!(mime_from_extension("py"), "application/x-python");
    }

    #[test]
    fn keeps_non_rig_attachments_typed_without_claiming_rig_support() {
        assert_eq!(
            mime_from_extension("docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(mime_from_extension("unknown"), "application/octet-stream");
    }
}
