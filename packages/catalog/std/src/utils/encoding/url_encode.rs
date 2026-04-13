use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct UrlEncodeNode {}

impl UrlEncodeNode {
    pub fn new() -> Self {
        UrlEncodeNode {}
    }
}

#[async_trait]
impl NodeLogic for UrlEncodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_url_encode",
            "URL Encode",
            "Percent-encodes a string for safe use in URLs (RFC 3986)",
            "Utils/Encoding",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "String to encode", VariableType::String);
        node.add_output_pin(
            "output",
            "Encoded",
            "URL-encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let encoded = urlencoding::encode(&input).into_owned();
        context.set_pin_value("output", json!(encoded)).await?;
        Ok(())
    }
}
