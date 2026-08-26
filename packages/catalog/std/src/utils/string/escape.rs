use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringEscapeNode {}

impl StringEscapeNode {
    pub fn new() -> Self {
        StringEscapeNode {}
    }
}

#[async_trait]
impl NodeLogic for StringEscapeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_escape",
            "Escape String",
            "Escapes special characters in a string (newlines, tabs, carriage returns, backslashes, quotes).",
            "Utils/String",
        );
        node.set_flowscript_name("string", "escape");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin("string", "String", "Input String", VariableType::String);

        node.add_output_pin(
            "escaped",
            "Escaped",
            "String with special characters escaped",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;

        let escaped = string
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
            .replace('"', "\\\"");

        context.set_pin_value("escaped", json!(escaped)).await?;
        Ok(())
    }
}
