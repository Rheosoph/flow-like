use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::collections::HashSet;
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct MakeSetNode {}

impl MakeSetNode {
    pub fn new() -> Self {
        MakeSetNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new("make_set", "Make Set", "Creates an empty set", "Utils/Set");

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_output_pin("set_out", "Set", "The created set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let set_out: HashSet<flow_like_types::Value> = HashSet::new();
        context.set_pin_value("set_out", json!(set_out)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_out", board, Some(ValueType::HashSet), None);
    }
}
