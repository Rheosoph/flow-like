use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct UrlDecodeNode {}

impl UrlDecodeNode {
    pub fn new() -> Self {
        UrlDecodeNode {}
    }
}

#[async_trait]
impl NodeLogic for UrlDecodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_url_decode",
            "URL Decode",
            "Decodes a percent-encoded URL string back to plain text",
            "Utils/Encoding",
        );
        node.set_flowscript_name("encoding", "urlDecode");
        node.set_receiver("input");
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "URL-encoded string", VariableType::String);
        node.add_output_pin("output", "Decoded", "Decoded string", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let decoded = urlencoding::decode(&input)?.into_owned();
        context.set_pin_value("output", json!(decoded)).await?;
        Ok(())
    }
}
