use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct AbsFloatNode {}

impl AbsFloatNode {
    pub fn new() -> Self {
        AbsFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for AbsFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_abs",
            "Abs",
            "Calculates the absolute value of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "abs");
        node.set_receiver("float");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float", "Float", "Input Float", VariableType::Float);

        node.add_output_pin(
            "absolute",
            "Absolute",
            "The absolute value of the float",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float: f64 = context.evaluate_pin("float").await?;

        let absolute = float.abs();

        context.set_pin_value("absolute", json!(absolute)).await?;
        Ok(())
    }
}
