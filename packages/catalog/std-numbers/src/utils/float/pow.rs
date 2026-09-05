use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PowerFloatNode {}

impl PowerFloatNode {
    pub fn new() -> Self {
        PowerFloatNode {}
    }
}

#[async_trait]
impl NodeLogic for PowerFloatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_power",
            "Power",
            "Calculates the power of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "power");
        node.set_receiver("base");
        node.add_icon("/flow/icons/sigma.svg");

        node.add_input_pin("base", "Base", "Base float", VariableType::Float);
        node.add_input_pin(
            "exponent",
            "Exponent",
            "Exponent float",
            VariableType::Float,
        );

        node.add_output_pin(
            "power",
            "Power",
            "Result of the power calculation",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let base: f64 = context.evaluate_pin("base").await?;
        let exponent: f64 = context.evaluate_pin("exponent").await?;

        let power = base.powf(exponent);
        context.set_pin_value("power", json!(power)).await?;

        Ok(())
    }
}
