use super::element_utils::extract_element_id;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Value, async_trait, json::json};
use std::time::Duration;

/// Sets a media component source from a FlowPath.
///
/// The node signs the FlowPath internally and lets the frontend update the target element
/// according to its component type. `filePreview` can then render image, video, audio, PDF,
/// text, and code files from the same source pin.
#[crate::register_node]
#[derive(Default)]
pub struct SetMediaSource;

impl SetMediaSource {
    pub fn new() -> Self {
        Self
    }
}

fn mime_from_extension(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "ts" | "tsx" => "text/typescript",
        "txt" => "text/plain",
        "md" | "mdx" => "text/markdown",
        "csv" => "text/csv",
        "toml" => "text/toml",
        "yaml" | "yml" => "text/yaml",
        _ => "application/octet-stream",
    }
}

fn media_kind_from_mime(mime_type: &str) -> &'static str {
    if mime_type.starts_with("image/") {
        return "image";
    }
    if mime_type.starts_with("video/") {
        return "video";
    }
    if mime_type.starts_with("audio/") {
        return "audio";
    }
    if mime_type == "application/pdf" {
        return "pdf";
    }
    if mime_type.contains("json")
        || mime_type.contains("javascript")
        || mime_type.contains("typescript")
    {
        return "code";
    }
    if mime_type.starts_with("text/") {
        return "text";
    }
    "file"
}

#[async_trait]
impl NodeLogic for SetMediaSource {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_media_source",
            "Set Media Source",
            "Signs a FlowPath and sets it as the source for image, video, avatar, iframe, lottie, or file preview elements",
            "UI/Elements/Media",
        );
        node.set_flowscript_name("ui", "setMediaSource");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the media element",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.add_input_pin(
            "file",
            "File",
            "FlowPath to sign and use as the element source",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "expiration",
            "Expiration (seconds)",
            "Expiration time for the signed URL",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3600)));

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);
        node.add_output_pin(
            "signed_url",
            "Signed URL",
            "The generated signed URL",
            VariableType::String,
        );
        node.add_output_pin(
            "mime_type",
            "MIME Type",
            "Detected MIME type from the FlowPath extension",
            VariableType::String,
        );
        node.add_output_pin(
            "media_kind",
            "Media Kind",
            "Detected media kind: image, video, audio, pdf, text, or file",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;
        let file: FlowPath = context.evaluate_pin("file").await?;
        let expiration: i64 = context.evaluate_pin("expiration").await?;

        let runtime_path = file.to_runtime(context).await?;
        let signed_url = runtime_path
            .store
            .sign(
                "GET",
                &runtime_path.path,
                Duration::from_secs(expiration.max(1) as u64),
            )
            .await?;

        let filename = runtime_path.path.filename().map(|name| name.to_string());
        let extension = runtime_path
            .path
            .extension()
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();
        let mime_type = mime_from_extension(&extension).to_string();
        let media_kind = media_kind_from_mime(&mime_type).to_string();

        let update = json!({
            "type": "setMediaSource",
            "src": signed_url.to_string(),
            "url": signed_url.to_string(),
            "filename": filename,
            "mimeType": mime_type,
            "mediaKind": media_kind
        });

        context.upsert_element(&element_id, update).await?;
        context
            .set_pin_value("signed_url", json!(signed_url.to_string()))
            .await?;
        context.set_pin_value("mime_type", json!(mime_type)).await?;
        context
            .set_pin_value("media_kind", json!(media_kind))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
