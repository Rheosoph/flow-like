use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    base64::{Engine, engine::general_purpose},
    json::json,
};

#[crate::register_node]
#[derive(Default)]
pub struct Base64EncodeNode {}

impl Base64EncodeNode {
    pub fn new() -> Self {
        Base64EncodeNode {}
    }
}

#[async_trait]
impl NodeLogic for Base64EncodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_base64_encode",
            "Base64 Encode",
            "Encodes a string to Base64",
            "Utils/Encoding",
        );
        node.set_flowscript_name("encoding", "base64Encode");
        node.set_receiver("input");
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "String to encode", VariableType::String);
        node.add_output_pin(
            "output",
            "Encoded",
            "Base64 encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let encoded = general_purpose::STANDARD.encode(input.as_bytes());
        context.set_pin_value("output", json!(encoded)).await?;
        Ok(())
    }
}
