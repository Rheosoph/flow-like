use super::ops::{bytes_input, bytes_node};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::{looks_like_text, sniff_file_type as sniff};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BytesDetectTypeNode {}

impl BytesDetectTypeNode {
    pub fn new() -> Self {
        BytesDetectTypeNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesDetectTypeNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_detect_type",
            "Detect Type",
            "Reads the leading bytes to work out what kind of file a buffer holds",
        );
        node.set_flowscript_name("bytes", "detectType");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");

        node.add_output_pin(
            "mime_type",
            "MIME Type",
            "Detected media type, empty when nothing matched",
            VariableType::String,
        );
        node.add_output_pin(
            "extension",
            "Extension",
            "Usual file extension for the detected type",
            VariableType::String,
        );
        node.add_output_pin(
            "detected",
            "Detected",
            "True when a signature matched",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "is_text",
            "Is Text",
            "True when the first kilobyte reads as UTF-8 text without null bytes",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes = context.evaluate_pin_bytes("bytes").await?;

        let detected = sniff(&bytes);
        let is_text = detected.is_none() && looks_like_text(&bytes);
        let (mime, extension) = match detected {
            Some((mime, extension)) => (mime.to_string(), extension.to_string()),
            None if is_text => ("text/plain".to_string(), "txt".to_string()),
            None => (String::new(), String::new()),
        };

        context.set_pin_value("mime_type", json!(mime)).await?;
        context.set_pin_value("extension", json!(extension)).await?;
        context
            .set_pin_value("detected", json!(detected.is_some()))
            .await?;
        context.set_pin_value("is_text", json!(is_text)).await?;
        Ok(())
    }
}
