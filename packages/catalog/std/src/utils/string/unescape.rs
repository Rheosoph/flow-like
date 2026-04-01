use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringUnescapeNode {}

impl StringUnescapeNode {
    pub fn new() -> Self {
        StringUnescapeNode {}
    }
}

#[async_trait]
impl NodeLogic for StringUnescapeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_unescape",
            "Unescape String",
            "Unescapes special character sequences in a string (\\n, \\t, \\r, \\\\, \\\").",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin("string", "String", "Input String", VariableType::String);

        node.add_output_pin(
            "unescaped",
            "Unescaped",
            "String with escape sequences resolved to actual characters",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;

        let mut result = String::with_capacity(string.len());
        let mut chars = string.chars();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some('\\') => result.push('\\'),
                    Some('"') => result.push('"'),
                    Some(other) => {
                        result.push('\\');
                        result.push(other);
                    }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }

        context.set_pin_value("unescaped", json!(result)).await?;
        Ok(())
    }
}
