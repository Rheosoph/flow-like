use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ClampFloatNode {}

impl ClampFloatNode {
    pub fn new() -> Self {
        ClampFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for ClampFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_clamp",
            "Clamp",
            "Clamps a float within a given range",
            "Math/Float",
        );
        node.set_flowscript_name("float", "clamp");
        node.set_receiver("float");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float", "Float", "Input Float", VariableType::Float);
        node.add_input_pin("min", "Min", "Minimum Value", VariableType::Float);
        node.add_input_pin("max", "Max", "Maximum Value", VariableType::Float);

        node.add_output_pin(
            "clamped",
            "Clamped",
            "The clamped float",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float: f64 = context.evaluate_pin("float").await?;
        let min: f64 = context.evaluate_pin("min").await?;
        let max: f64 = context.evaluate_pin("max").await?;

        let clamped = float.clamp(min, max);

        context.set_pin_value("clamped", json!(clamped)).await?;
        Ok(())
    }
}
