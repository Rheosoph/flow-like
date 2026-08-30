use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

/// URL-decodes a percent-encoded string.
#[crate::register_node]
#[derive(Default)]
pub struct UrlDecode;

impl UrlDecode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UrlDecode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_url_decode",
            "URL Decode",
            "Decodes a URL-encoded (percent-encoded) string",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "urlDecode");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "input",
            "Input",
            "The URL-encoded string to decode",
            VariableType::String,
        );

        node.add_output_pin(
            "decoded",
            "Decoded",
            "The decoded string",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the decoding was successful",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;

        match urlencoding::decode(&input) {
            Ok(decoded) => {
                context
                    .set_pin_value("decoded", json::json!(decoded.to_string()))
                    .await?;
                context.set_pin_value("success", json::json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("decoded", json::json!(input)).await?;
                context.set_pin_value("success", json::json!(false)).await?;
            }
        }

        Ok(())
    }
}
