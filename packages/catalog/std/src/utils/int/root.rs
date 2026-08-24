use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RootIntegerNode {}

impl RootIntegerNode {
    pub fn new() -> Self {
        RootIntegerNode {}
    }
}

#[async_trait]
impl NodeLogic for RootIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_root",
            "Root",
            "Calculates the nth root of an integer",
            "Math/Int",
        );
        node.set_flowscript_name("int", "root");
        node.set_receiver("radicand");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin(
            "radicand",
            "Radicand",
            "The integer to take the root of",
            VariableType::Integer,
        );
        node.add_input_pin(
            "degree",
            "Degree",
            "The degree of the root",
            VariableType::Integer,
        );

        node.add_output_pin(
            "root",
            "Root",
            "Result of the root calculation",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let radicand: i64 = context.evaluate_pin("radicand").await?;
        let degree: i64 = context.evaluate_pin("degree").await?;

        if degree <= 0 {
            context.log_message("Degree must be a positive integer", LogLevel::Error);
            context.set_pin_value("root", json!(0.0)).await?;
            return Ok(());
        }

        let root = (radicand as f64).powf(1.0 / degree as f64);
        context.set_pin_value("root", json!(root)).await?;

        Ok(())
    }
}
