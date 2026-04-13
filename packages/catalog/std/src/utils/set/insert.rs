use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::{collections::HashSet, sync::Arc};

#[crate::register_node]
#[derive(Default)]
pub struct InsertSetNode {}

impl InsertSetNode {
    pub fn new() -> Self {
        InsertSetNode {}
    }
}

#[async_trait]
impl NodeLogic for InsertSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "insert",
            "Insert Element",
            "Inserts an element to the set",
            "Utils/Set",
        );

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("set_in", "Set", "Your Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_input_pin("value", "Value", "Value to push", VariableType::Generic);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin("set_out", "Set", "Adjusted Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_output_pin(
            "existed_before",
            "Existed Before?",
            "Was the element there before?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut set_in: HashSet<Value> = context.evaluate_pin("set_in").await?;
        let element: Value = context.evaluate_pin("value").await?;
        let was_there_before = set_in.insert(element);
        context.set_pin_value("set_out", json!(set_in)).await?;
        context
            .set_pin_value("existed_before", json!(was_there_before))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_out", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("set_in", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        let _ = node.match_type("existed_before", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["set_in", "set_out", "value", "existed_before"], true);
    }
}
