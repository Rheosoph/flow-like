use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ModuloIntegerNode {}

impl ModuloIntegerNode {
    pub fn new() -> Self {
        ModuloIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for ModuloIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_modulo",
            "% (Int)",
            "Calculates the remainder of integer division",
            "Math/Int",
        );
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("integer1", "Integer 1", "Dividend", VariableType::Integer);
        node.add_input_pin("integer2", "Integer 2", "Divisor", VariableType::Integer);

        node.add_output_pin(
            "remainder",
            "Remainder",
            "Remainder of the division",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;

        if integer2 == 0 {
            context.set_pin_value("remainder", json!(0)).await?;
            context.log_message("Divided by Zero", LogLevel::Error);
        } else {
            let remainder = integer1 % integer2;
            context.set_pin_value("remainder", json!(remainder)).await?;
        }

        Ok(())
    }
}
