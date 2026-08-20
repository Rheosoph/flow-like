use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SubtractFloatNode {}

impl SubtractFloatNode {
    pub fn new() -> Self {
        SubtractFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for SubtractFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_subtract",
            "- (Float)",
            "Subtracts one float from another",
            "Math/Float",
        );
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float1", "Float 1", "First Float", VariableType::Float);
        node.add_input_pin("float2", "Float 2", "Second Float", VariableType::Float);

        node.add_output_pin(
            "difference",
            "Difference",
            "The difference between the two floats",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float1: f64 = context.evaluate_pin("float1").await?;
        let float2: f64 = context.evaluate_pin("float2").await?;

        let difference = float1 - float2;

        context
            .set_pin_value("difference", json!(difference))
            .await?;
        Ok(())
    }
}
