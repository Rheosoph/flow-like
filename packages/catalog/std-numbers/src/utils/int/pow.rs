use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PowerIntegerNode {}

impl PowerIntegerNode {
    pub fn new() -> Self {
        PowerIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for PowerIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_power",
            "Power",
            "Calculates the power of an integer",
            "Math/Int",
        );
        node.set_flowscript_name("int", "power");
        node.set_receiver("base");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("base", "Base", "Base integer", VariableType::Integer);
        node.add_input_pin(
            "exponent",
            "Exponent",
            "Exponent integer",
            VariableType::Integer,
        );

        node.add_output_pin(
            "power",
            "Power",
            "Result of the power calculation",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let base: i64 = context.evaluate_pin("base").await?;
        let exponent: i64 = context.evaluate_pin("exponent").await?;

        let power = base.pow(exponent as u32);
        context.set_pin_value("power", json!(power)).await?;

        Ok(())
    }
}
