use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    base64::{Engine, engine::general_purpose},
    json::json,
};

#[crate::register_node]
#[derive(Default)]
pub struct Base64DecodeBytesNode {}

impl Base64DecodeBytesNode {
    pub fn new() -> Self {
        Base64DecodeBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for Base64DecodeBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_base64_decode_bytes",
            "Base64 Decode to Bytes",
            "Decodes a Base64 string to raw bytes",
            "Utils/Encoding",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin(
            "input",
            "Input",
            "Base64 encoded string",
            VariableType::String,
        );
        node.add_output_pin(
            "output",
            "Decoded",
            "Decoded raw bytes",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let bytes = general_purpose::STANDARD.decode(&input)?;
        context.set_pin_value("output", json!(bytes)).await?;
        Ok(())
    }
}
