use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct AbsoluteIntegerNode {}

impl AbsoluteIntegerNode {
    pub fn new() -> Self {
        AbsoluteIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for AbsoluteIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_abs",
            "Absolute",
            "Returns the absolute value of an Integer",
            "Math/Int",
        );
        node.set_flowscript_name("int", "abs");
        node.set_receiver("integer");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("integer", "Integer", "Input Integer", VariableType::Integer);

        node.add_output_pin(
            "absolute",
            "Absolute",
            "Absolute Value",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer: i64 = context.evaluate_pin("integer").await?;
        let absolute = integer.abs();
        context.set_pin_value("absolute", json!(absolute)).await?;
        Ok(())
    }
}
