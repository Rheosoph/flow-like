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
pub struct Base64DecodeNode {}

impl Base64DecodeNode {
    pub fn new() -> Self {
        Base64DecodeNode {}
    }
}

#[async_trait]
impl NodeLogic for Base64DecodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_base64_decode",
            "Base64 Decode",
            "Decodes a Base64 string back to a UTF-8 string",
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
            "Decoded UTF-8 string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let bytes = general_purpose::STANDARD.decode(&input)?;
        let decoded = String::from_utf8(bytes)?;
        context.set_pin_value("output", json!(decoded)).await?;
        Ok(())
    }
}
