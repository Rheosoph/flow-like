use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct AddIntegerNode {}

impl AddIntegerNode {
    pub fn new() -> Self {
        AddIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for AddIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new("int_add", "+ (Int)", "Adds two Integers", "Math/Int");
        node.set_flowscript_name("int", "add");
        node.set_receiver("integer1");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin(
            "integer1",
            "Integer 1",
            "Input Integer",
            VariableType::Integer,
        );
        node.add_input_pin(
            "integer2",
            "Integer 2",
            "Input Integer",
            VariableType::Integer,
        );

        node.add_output_pin(
            "sum",
            "Sum",
            "Sum of the two integers",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;
        let sum = integer1 + integer2;
        context.set_pin_value("sum", json!(sum)).await?;
        Ok(())
    }
}
