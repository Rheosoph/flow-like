use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct SelectNode {}

impl SelectNode {
    pub fn new() -> Self {
        SelectNode {}
    }
}

#[async_trait]
impl NodeLogic for SelectNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_types_select",
            "Select",
            "Selects between two values based on a boolean condition. Returns A if true, B if false.",
            "Utils/Types",
        );
        node.add_icon("/flow/icons/split.svg");

        node.add_input_pin(
            "a",
            "A (True)",
            "Value returned when condition is true",
            VariableType::Generic,
        );

        node.add_input_pin(
            "b",
            "B (False)",
            "Value returned when condition is false",
            VariableType::Generic,
        );

        node.add_input_pin(
            "condition",
            "Condition",
            "If true, returns A. If false, returns B.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "result",
            "Result",
            "The selected value (A if true, B if false)",
            VariableType::Generic,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let condition: bool = context.evaluate_pin("condition").await?;

        let result: Value = if condition {
            context.evaluate_pin("a").await?
        } else {
            context.evaluate_pin("b").await?
        };

        context.set_pin_value("result", result).await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("a", board, None, None);
        let _ = node.match_type("b", board, None, None);
        let _ = node.match_type("result", board, None, None);
        node.harmonize_type(vec!["a", "b", "result"], true);
        node.harmonize_value_type(vec!["a", "b", "result"]);
    }
}
