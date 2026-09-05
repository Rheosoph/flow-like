use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RootFloatNode {}

impl RootFloatNode {
    pub fn new() -> Self {
        RootFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for RootFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_root",
            "Root",
            "Calculates the nth root of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "root");
        node.set_receiver("radicand");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin(
            "radicand",
            "Radicand",
            "The float to take the root of",
            VariableType::Float,
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
        let radicand: f64 = context.evaluate_pin("radicand").await?;
        let degree: i64 = context.evaluate_pin("degree").await?;

        if degree <= 0 {
            context.log_message("Degree must be a positive integer", LogLevel::Error);
            context.set_pin_value("root", json!(0.0)).await?;
            return Ok(());
        }

        let root = radicand.powf(1.0 / degree as f64);
        context.set_pin_value("root", json!(root)).await?;

        Ok(())
    }
}
