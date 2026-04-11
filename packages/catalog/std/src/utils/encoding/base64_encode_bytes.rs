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
pub struct Base64EncodeBytesNode {}

impl Base64EncodeBytesNode {
    pub fn new() -> Self {
        Base64EncodeBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for Base64EncodeBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_base64_encode_bytes",
            "Base64 Encode Bytes",
            "Encodes raw bytes to a Base64 string",
            "Utils/Encoding",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "Raw bytes to encode", VariableType::Byte)
            .set_value_type(ValueType::Array);
        node.add_output_pin(
            "output",
            "Encoded",
            "Base64 encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: Vec<u8> = context.evaluate_pin("input").await?;
        let encoded = general_purpose::STANDARD.encode(&input);
        context.set_pin_value("output", json!(encoded)).await?;
        Ok(())
    }
}
