use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MinFloatNode {}

impl MinFloatNode {
    pub fn new() -> Self {
        MinFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for MinFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_min",
            "Min",
            "Returns the smaller of two floats",
            "Math/Float",
        );
        node.set_flowscript_name("float", "min");
        node.set_receiver("float1");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float1", "Float 1", "First Float", VariableType::Float);
        node.add_input_pin("float2", "Float 2", "Second Float", VariableType::Float);

        node.add_output_pin(
            "minimum",
            "Minimum",
            "The smaller of the two floats",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float1: f64 = context.evaluate_pin("float1").await?;
        let float2: f64 = context.evaluate_pin("float2").await?;

        let minimum = float1.min(float2);

        context.set_pin_value("minimum", json!(minimum)).await?;
        Ok(())
    }
}
