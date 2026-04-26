use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct SetHasNode {}

impl SetHasNode {
    pub fn new() -> Self {
        SetHasNode {}
    }
}

#[async_trait]
impl NodeLogic for SetHasNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_has",
            "Has Element",
            "Checks if an element is present in the set",
            "Utils/Set",
        );

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("set_in", "Set", "Your Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_input_pin(
            "value",
            "Value",
            "Value to search for",
            VariableType::Generic,
        );

        node.add_output_pin(
            "contains",
            "Contains?",
            "Does the set include the value?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let set_in: &HashSet<Value> = &context.evaluate_pin("set_in").await?;
        let value: Value = context.evaluate_pin("value").await?;
        let includes = set_in.contains(&value);

        context.set_pin_value("contains", json!(includes)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_in", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["set_in", "value"], true);
    }
}
