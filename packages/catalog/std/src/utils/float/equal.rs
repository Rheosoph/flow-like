use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct EqualFloatNode {}

impl EqualFloatNode {
    pub fn new() -> Self {
        EqualFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for EqualFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_equal",
            "== (Float)",
            "Checks if two floats are equal (within a tolerance)",
            "Math/Float/Comparison",
        );
        node.set_flowscript_name("float", "equal");
        node.set_receiver("float1");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_version(1);

        node.add_input_pin("float1", "Float 1", "First Float", VariableType::Float);
        node.add_input_pin("float2", "Float 2", "Second Float", VariableType::Float);
        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Comparison Tolerance",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));

        node.add_output_pin(
            "is_equal",
            "Is Equal",
            "True if the floats are equal, false otherwise",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float1: f64 = context.evaluate_pin("float1").await?;
        let float2: f64 = context.evaluate_pin("float2").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;

        let is_equal = (float1 - float2).abs() <= tolerance;

        context.set_pin_value("is_equal", json!(is_equal)).await?;
        Ok(())
    }
}
