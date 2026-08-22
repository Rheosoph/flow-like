use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FloatToIntNode {}

impl FloatToIntNode {
    pub fn new() -> Self {
        FloatToIntNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatToIntNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_to_int",
            "To Integer",
            "Converts a float into an integer using the selected rounding",
            "Math/Float",
        );
        node.add_icon("/flow/icons/convert.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("float", "Float", "Input Float", VariableType::Float);
        node.add_input_pin(
            "rounding",
            "Rounding",
            "How to remove the fractional part",
            VariableType::String,
        )
        .set_default_value(Some(json!("Round")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Round".to_string(),
                    "Floor".to_string(),
                    "Ceil".to_string(),
                    "Truncate".to_string(),
                ])
                .build(),
        );

        node.add_output_pin(
            "integer",
            "Integer",
            "The converted value",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float: f64 = context.evaluate_pin("float").await?;
        let rounding: String = context.evaluate_pin("rounding").await?;

        let rounded = match rounding.as_str() {
            "Floor" => float.floor(),
            "Ceil" => float.ceil(),
            "Truncate" => float.trunc(),
            _ => float.round(),
        };

        if !rounded.is_finite() {
            return Err(flow_like_types::anyhow!(
                "Cannot convert {float} into an integer"
            ));
        }

        context
            .set_pin_value(
                "integer",
                json!(rounded.clamp(i64::MIN as f64, i64::MAX as f64) as i64),
            )
            .await?;
        Ok(())
    }
}
