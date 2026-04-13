use ahash::HashSet;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail, json::json};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct PopSetNode {}

impl PopSetNode {
    pub fn new() -> Self {
        PopSetNode {}
    }
}

#[async_trait]
impl NodeLogic for PopSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_pop",
            "Pop",
            "Pops a random element of a set",
            "Utils/Set",
        );
        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("set_in", "Set", "Your Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin("set_out", "Set", "Adjusted Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let set_in: HashSet<Value> = context.evaluate_pin("set_in").await?;
        let mut set_out = set_in.clone();
        if let Some(elem) = set_in.iter().next() {
            set_out.remove(elem);
            context.set_pin_value("set_out", json!(set_out)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }
        bail!("Cannot remove an arbitrary element from a empty set")
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_in", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("set_out", board, Some(ValueType::HashSet), None);
        node.harmonize_type(vec!["set_in", "set_out"], true);
    }
}
