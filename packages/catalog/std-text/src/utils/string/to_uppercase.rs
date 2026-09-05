use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringToUpperNode {}

impl StringToUpperNode {
    pub fn new() -> Self {
        StringToUpperNode {}
    }
}

#[async_trait]
impl NodeLogic for StringToUpperNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_to_upper",
            "To Upper Case",
            "Converts a string to uppercase",
            "Utils/String",
        );
        node.set_flowscript_name("string", "toUpper");
        node.set_receiver("string");
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin("string", "String", "Input String", VariableType::String);

        node.add_output_pin(
            "uppercase_string",
            "Uppercase String",
            "String in uppercase",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let uppercase_string = string.to_uppercase();

        context
            .set_pin_value("uppercase_string", json!(uppercase_string))
            .await?;
        Ok(())
    }
}
