use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringEndsWithNode {}

impl StringEndsWithNode {
    pub fn new() -> Self {
        StringEndsWithNode {}
    }
}

#[async_trait]
impl NodeLogic for StringEndsWithNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_ends_with",
            "Ends With",
            "Checks if a string ends with a specific string",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_version(1);

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "suffix",
            "Suffix",
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
            "ends_with",
            "Ends With?",
            "Does the string end with the suffix?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let suffix: String = context.evaluate_pin("suffix").await?;
        let ignore_case: bool = context.evaluate_pin("ignore_case").await?;

        let ends_with = if ignore_case {
            string.to_lowercase().ends_with(&suffix.to_lowercase())
        } else {
            string.ends_with(&suffix)
        };

        context.set_pin_value("ends_with", json!(ends_with)).await?;
        Ok(())
    }
}
