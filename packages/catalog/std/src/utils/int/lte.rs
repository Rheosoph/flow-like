use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LessThanOrEqualIntegerNode {}

impl LessThanOrEqualIntegerNode {
    pub fn new() -> Self {
        LessThanOrEqualIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for LessThanOrEqualIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_less_than_or_equal",
            "<= (Int)",
            "Checks if the first integer is less than or equal to the second",
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
            "less_than_or_equal",
            "Less Than or Equal",
            "True if integer1 <= integer2, false otherwise",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;

        let less_than_or_equal = integer1 <= integer2;

        context
            .set_pin_value("less_than_or_equal", json!(less_than_or_equal))
            .await?;
        Ok(())
    }
}
