use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct IntLimitsNode {}

impl IntLimitsNode {
    pub fn new() -> Self {
        IntLimitsNode {}
    }
}

#[async_trait]
impl NodeLogic for IntLimitsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_limits",
            "Integer Limits",
            "The smallest and largest representable integer",
            "Math/Int",
        );
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_output_pin(
            "min",
            "Min",
            "Smallest representable integer",
            VariableType::Integer,
        );
        node.add_output_pin(
            "max",
            "Max",
            "Largest representable integer",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.set_pin_value("min", json!(i64::MIN)).await?;
        context.set_pin_value("max", json!(i64::MAX)).await?;
        Ok(())
    }
}
