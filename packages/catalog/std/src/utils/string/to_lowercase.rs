use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringToLowerNode {}

impl StringToLowerNode {
    pub fn new() -> Self {
        StringToLowerNode {}
    }
}

#[async_trait]
impl NodeLogic for StringToLowerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_to_lower",
            "To Lower Case",
            "Converts a string to lowercase",
            "Utils/String",
        );
        node.set_flowscript_name("string", "toLower");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin("string", "String", "Input String", VariableType::String);

        node.add_output_pin(
            "lowercase_string",
            "Lowercase String",
            "String in lowercase",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let lowercase_string = string.to_lowercase();

        context
            .set_pin_value("lowercase_string", json!(lowercase_string))
            .await?;
        Ok(())
    }
}
