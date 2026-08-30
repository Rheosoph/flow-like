use super::ops::{bytes_input, bytes_node, bytes_output};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BytesToTextNode {}

impl BytesToTextNode {
    pub fn new() -> Self {
        BytesToTextNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesToTextNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_to_text",
            "Bytes to Text",
            "Reads a byte buffer as UTF-8 text",
        );
        node.set_flowscript_name("bytes", "toText");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");
        node.add_input_pin(
            "lossy",
            "Lossy",
            "Replace invalid sequences instead of failing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("text", "Text", "The decoded text", VariableType::String);
        node.add_output_pin(
            "was_valid",
            "Was Valid",
            "False when the buffer was not valid UTF-8",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let lossy: bool = context.evaluate_pin("lossy").await?;

        let text = match String::from_utf8(bytes.clone()) {
            Ok(text) => {
                context.set_pin_value("was_valid", json!(true)).await?;
                text
            }
            Err(_) if lossy => {
                context.set_pin_value("was_valid", json!(false)).await?;
                String::from_utf8_lossy(&bytes).into_owned()
            }
            Err(error) => {
                return Err(flow_like_types::anyhow!(
                    "Bytes are not valid UTF-8: {error}"
                ));
            }
        };

        context.set_pin_value("text", json!(text)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TextToBytesNode {}

impl TextToBytesNode {
    pub fn new() -> Self {
        TextToBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for TextToBytesNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "text_to_bytes",
            "Text to Bytes",
            "Writes text out as UTF-8 bytes",
        );
        node.set_flowscript_name("bytes", "fromText");
        node.add_input_pin("text", "Text", "Input Text", VariableType::String);
        bytes_output(&mut node, "bytes", "Bytes", "The encoded bytes");

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let text: String = context.evaluate_pin("text").await?;
        context
            .set_pin_value("bytes", json!(text.into_bytes()))
            .await?;
        Ok(())
    }
}
