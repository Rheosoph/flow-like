use ahash::HashSet;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DiscardSetNode {}

impl DiscardSetNode {
    pub fn new() -> Self {
        DiscardSetNode {}
    }
}

#[async_trait]
impl NodeLogic for DiscardSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_discard",
            "Discard",
            "Discards an element of a set",
            "Utils/Set",
        );
        node.set_flowscript_name("set", "discard");
        node.set_receiver("set_in");
        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("set_in", "Set", "Your Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_input_pin("value", "Value", "Value to remove", VariableType::Generic);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin("set_out", "Set", "Adjusted Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_output_pin(
            "has_removed",
            "Was Removed",
            "If the element was removed",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let set_in: HashSet<Value> = context.evaluate_pin("set_in").await?;
        let element: Value = context.evaluate_pin("value").await?;
        let mut set_out = set_in.clone();
        let has_removed = set_out.remove(&element);

        context.set_pin_value("set_out", json!(set_out)).await?;
        context
            .set_pin_value("has_removed", json!(has_removed))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_out", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("set_in", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        let _ = node.match_type("has_removed", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["set_in", "value", "set_out", "has_removed"], true);
    }
}
