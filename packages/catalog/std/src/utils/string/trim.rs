use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringTrimNode {}

impl StringTrimNode {
    pub fn new() -> Self {
        StringTrimNode {}
    }
}

#[async_trait]
impl NodeLogic for StringTrimNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_trim",
            "Trim String",
            "Removes leading and trailing whitespace from a string",
            "Utils/String",
        );
        node.set_flowscript_name("string", "trim");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin("string", "String", "Input String", VariableType::String);

        node.add_output_pin(
            "trimmed_string",
            "Trimmed String",
            "String without leading/trailing whitespace",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let trimmed_string = string.trim().to_string();

        context
            .set_pin_value("trimmed_string", json!(trimmed_string))
            .await?;
        Ok(())
    }
}
