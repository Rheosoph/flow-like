use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

/// URL-encodes a string for safe use in URLs.
#[crate::register_node]
#[derive(Default)]
pub struct UrlEncode;

impl UrlEncode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UrlEncode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_url_encode",
            "URL Encode",
            "Encodes a string for safe use in URLs (percent-encoding)",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "urlEncode");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "input",
            "Input",
            "The string to URL-encode",
            VariableType::String,
        );

        node.add_output_pin(
            "encoded",
            "Encoded",
            "The URL-encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let encoded = urlencoding::encode(&input).to_string();
        context
            .set_pin_value("encoded", json::json!(encoded))
            .await?;
        Ok(())
    }
}
