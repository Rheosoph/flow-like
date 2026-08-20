use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct GreaterThanFloatNode {}

impl GreaterThanFloatNode {
    pub fn new() -> Self {
        GreaterThanFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for GreaterThanFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_greater_than",
            "> (Float)",
            "Checks if one float is greater than another",
            "Math/Float/Comparison",
        );
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float1", "Float 1", "First Float", VariableType::Float);
        node.add_input_pin("float2", "Float 2", "Second Float", VariableType::Float);

        node.add_output_pin(
            "is_greater",
            "Is Greater",
            "True if float1 is greater than float2, false otherwise",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float1: f64 = context.evaluate_pin("float1").await?;
        let float2: f64 = context.evaluate_pin("float2").await?;

        let is_greater = float1 > float2;

        context
            .set_pin_value("is_greater", json!(is_greater))
            .await?;
        Ok(())
    }
}
