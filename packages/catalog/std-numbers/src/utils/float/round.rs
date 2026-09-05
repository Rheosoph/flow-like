use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RoundFloatNode {}

impl RoundFloatNode {
    pub fn new() -> Self {
        RoundFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for RoundFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_round",
            "Round",
            "Rounds a float to the given number of decimal places",
            "Math/Float",
        );
        node.set_flowscript_name("float", "round");
        node.set_receiver("float");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_version(1);
        node.set_scores(pure_scores());

        node.add_input_pin("float", "Float", "Input Float", VariableType::Float);
        node.add_input_pin(
            "decimals",
            "Decimals",
            "Number of decimal places to keep",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "rounded",
            "Rounded",
            "The rounded float",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float: f64 = context.evaluate_pin("float").await?;
        let decimals: i64 = context.evaluate_pin("decimals").await?;

        let factor = 10f64.powi(decimals.clamp(0, 15) as i32);
        let rounded = (float * factor).round() / factor;

        context.set_pin_value("rounded", json!(rounded)).await?;
        Ok(())
    }
}
