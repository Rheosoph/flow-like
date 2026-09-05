use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DivideIntegerNode {}

impl DivideIntegerNode {
    pub fn new() -> Self {
        DivideIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for DivideIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_divide",
            "/ (Int)",
            "Divides two Integers (handles division by zero)",
            "Math/Int",
        );
        node.set_flowscript_name("int", "divide");
        node.set_receiver("integer1");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("integer1", "Integer 1", "Dividend", VariableType::Integer);
        node.add_input_pin("integer2", "Integer 2", "Divisor", VariableType::Integer);

        node.add_output_pin(
            "result",
            "Result",
            "Result of the division",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let integer1: i64 = context.evaluate_pin("integer1").await?;
        let integer2: i64 = context.evaluate_pin("integer2").await?;

        if integer2 == 0 {
            context.set_pin_value("result", json!(0.0)).await?;
            context.log_message("Divided by Zero", LogLevel::Error);
        } else {
            let result = integer1 as f64 / integer2 as f64;
            context.set_pin_value("result", json!(result)).await?;
        }

        Ok(())
    }
}
