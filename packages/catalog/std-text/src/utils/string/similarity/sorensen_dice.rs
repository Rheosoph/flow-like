use strsim::sorensen_dice;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SorensenDiceCoefficientNode {}

impl SorensenDiceCoefficientNode {
    pub fn new() -> Self {
        SorensenDiceCoefficientNode {}
    }
}

#[async_trait]
impl NodeLogic for SorensenDiceCoefficientNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "sorensen_dice_coefficient",
            "Sørensen-Dice Coefficient",
            "Calculates the Sørensen-Dice coefficient between two strings",
            "Utils/String/Similarity",
        );
        node.set_flowscript_name("string", "sorensenDiceCoefficient");
        node.set_receiver("string1");
        node.add_icon("/flow/icons/distance.svg");

        node.add_input_pin("string1", "String 1", "First String", VariableType::String);
        node.add_input_pin("string2", "String 2", "Second String", VariableType::String);

        node.add_output_pin(
            "coefficient",
            "Coefficient",
            "Sørensen-Dice Coefficient",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string1: String = context.evaluate_pin("string1").await?;
        let string2: String = context.evaluate_pin("string2").await?;

        let coefficient = sorensen_dice(&string1, &string2);

        context
            .set_pin_value("coefficient", json!(coefficient))
            .await?;

        Ok(())
    }
}
