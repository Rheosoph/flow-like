use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FloorFloatNode {}

impl FloorFloatNode {
    pub fn new() -> Self {
        FloorFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for FloorFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_floor",
            "Floor",
            "Rounds a float down to the nearest integer",
            "Math/Float",
        );
        node.set_flowscript_name("float", "floor");
        node.set_receiver("float");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float", "Float", "Input Float", VariableType::Float);

        node.add_output_pin(
            "floor",
            "Floor",
            "The floor of the float",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float: f64 = context.evaluate_pin("float").await?;

        let floor = float.floor() as i64;

        context.set_pin_value("floor", json!(floor)).await?;
        Ok(())
    }
}
