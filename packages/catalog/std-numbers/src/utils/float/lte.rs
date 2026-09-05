use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LessThanOrEqualFloatNode {}

impl LessThanOrEqualFloatNode {
    pub fn new() -> Self {
        LessThanOrEqualFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for LessThanOrEqualFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_less_than_or_equal",
            "<= (Float)",
            "Checks if one float is less than or equal to another",
            "Math/Float/Comparison",
        );
        node.set_flowscript_name("float", "lessThanOrEqual");
        node.set_receiver("float1");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("float1", "Float 1", "First Float", VariableType::Float);
        node.add_input_pin("float2", "Float 2", "Second Float", VariableType::Float);

        node.add_output_pin(
            "is_less_or_equal",
            "Is Less or Equal",
            "True if float1 is less than or equal to float2, false otherwise",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float1: f64 = context.evaluate_pin("float1").await?;
        let float2: f64 = context.evaluate_pin("float2").await?;

        let is_less_or_equal = float1 <= float2;

        context
            .set_pin_value("is_less_or_equal", json!(is_less_or_equal))
            .await?;
        Ok(())
    }
}
