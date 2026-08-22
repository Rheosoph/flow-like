use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringStartsWithNode {}

impl StringStartsWithNode {
    pub fn new() -> Self {
        StringStartsWithNode {}
    }
}

#[async_trait]
impl NodeLogic for StringStartsWithNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_starts_with",
            "Starts With",
            "Checks if a string starts with a specific string",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_version(1);

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "prefix",
            "Prefix",
            "String to check against",
            VariableType::String,
        );

        node.add_input_pin(
            "ignore_case",
            "Ignore Case",
            "Compare without regard to upper/lower case",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "starts_with",
            "Starts With?",
            "Does the string start with the prefix?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let prefix: String = context.evaluate_pin("prefix").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let starts_with = if ignore_case {
            string.to_lowercase().starts_with(&prefix.to_lowercase())
        } else {
            string.starts_with(&prefix)
        };

        context
            .set_pin_value("starts_with", json!(starts_with))
            .await?;
        Ok(())
    }
}
