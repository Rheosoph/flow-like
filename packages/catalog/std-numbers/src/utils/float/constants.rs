use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

const CONSTANTS: [(&str, f64); 8] = [
    ("Pi", std::f64::consts::PI),
    ("Tau", std::f64::consts::TAU),
    ("E", std::f64::consts::E),
    ("Sqrt 2", std::f64::consts::SQRT_2),
    ("Ln 2", std::f64::consts::LN_2),
    ("Ln 10", std::f64::consts::LN_10),
    ("Epsilon", f64::EPSILON),
    ("Max", f64::MAX),
];

#[crate::register_node]
#[derive(Default)]
pub struct FloatConstantNode {}

impl FloatConstantNode {
    pub fn new() -> Self {
        FloatConstantNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatConstantNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_constant",
            "Math Constant",
            "Provides a mathematical constant such as Pi or E",
            "Math/Float",
        );
        node.set_flowscript_name("float", "constant");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin(
            "constant",
            "Constant",
            "Which constant to emit",
            VariableType::String,
        )
        .set_default_value(Some(json!("Pi")))
        .set_options(
            PinOptions::new()
                .set_valid_values(CONSTANTS.iter().map(|(name, _)| name.to_string()).collect())
                .build(),
        );

        node.add_output_pin(
            "value",
            "Value",
            "The value of the constant",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let constant: String = context.evaluate_pin("constant").await?;

        let value = CONSTANTS
            .iter()
            .find(|(name, _)| *name == constant)
            .map(|(_, value)| *value)
            .ok_or_else(|| flow_like_types::anyhow!("Unknown math constant {constant}"))?;

        context.set_pin_value("value", json!(value)).await?;
        Ok(())
    }
}
