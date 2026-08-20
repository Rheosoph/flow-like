use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct EqualIntegerNode {}

impl EqualIntegerNode {
    pub fn new() -> Self {
        EqualIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for EqualIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_equal",
            "== (Int)",
            "Checks if two integers are equal",
            "Math/Int",
        );
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin(
            "integer1",
            "Integer 1",
            "Input Integer",
            VariableType::Integer,
        );
        node.add_input_pin(
            "integer2",
            "Integer 2",
            "Input Integer",
            VariableType::Integer,
        );

        node.add_output_pin(
            "equal",
            "Equal",
            "True if the integers are equal, false otherwise",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;

        let equal = integer1 == integer2;

        context.set_pin_value("equal", json!(equal)).await?;
        Ok(())
    }
}
