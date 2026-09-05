use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use regex::Regex;

#[crate::register_node]
#[derive(Default)]
pub struct StringReplaceNode {}

impl StringReplaceNode {
    pub fn new() -> Self {
        StringReplaceNode {}
    }
}

#[async_trait]
impl NodeLogic for StringReplaceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_replace",
            "Replace String",
            "Replaces occurrences of a substring or regex pattern within a string.",
            "Utils/String",
        );
        node.set_flowscript_name("string", "replace");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");
        node.set_version(1);

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "pattern",
            "Pattern",
            "Substring or regex pattern to replace",
            VariableType::String,
        );
        node.add_input_pin(
            "replacement",
            "Replacement",
            "Replacement string (supports $1, $2, ... for regex capture groups)",
            VariableType::String,
        );
        node.add_input_pin(
            "is_regex",
            "Is Regex",
            "Treat the pattern as a regular expression",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "new_string",
            "New String",
            "String with replacements",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let pattern: String = context.evaluate_pin("pattern").await?;
        let replacement: String = context.evaluate_pin("replacement").await?;
        let is_regex: bool = context.evaluate_pin("is_regex").await?;

        let new_string = if is_regex {
            let re = Regex::new(&pattern)?;
            re.replace_all(&string, replacement.as_str()).into_owned()
        } else {
            string.replace(&pattern, &replacement)
        };

        context
            .set_pin_value("new_string", json!(new_string))
            .await?;
        Ok(())
    }
}
