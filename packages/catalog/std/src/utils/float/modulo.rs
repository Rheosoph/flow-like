use crate::utils::pure_scores;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FloatModuloNode {}

impl FloatModuloNode {
    pub fn new() -> Self {
        FloatModuloNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatModuloNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_modulo",
            "% (Float)",
            "Remainder of a float division",
            "Math/Float",
        );
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("float1", "Float 1", "Dividend", VariableType::Float);
        node.add_input_pin("float2", "Float 2", "Divisor", VariableType::Float);
        node.add_input_pin(
            "euclidean",
            "Always Positive",
            "Return a non-negative remainder",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "remainder",
            "Remainder",
            "Remainder of the division",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float1: f64 = context.evaluate_pin("float1").await?;
        let float2: f64 = context.evaluate_pin("float2").await?;
        let euclidean: bool = context.evaluate_pin("euclidean").await?;

        if float2 == 0.0 {
            context.log_message("Divided by Zero", LogLevel::Error);
            context.set_pin_value("remainder", json!(0.0)).await?;
            return Ok(());
        }

        let remainder = if euclidean {
            float1.rem_euclid(float2)
        } else {
            float1 % float2
        };

        context.set_pin_value("remainder", json!(remainder)).await?;
        Ok(())
    }
}
