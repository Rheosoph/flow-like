use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::HashSet;
use std::collections::hash_set::Difference;
use std::hash::RandomState;

#[crate::register_node]
#[derive(Default)]
pub struct DifferenceSetNode {}

impl DifferenceSetNode {
    pub fn new() -> Self {
        DifferenceSetNode {}
    }
}

#[async_trait]
impl NodeLogic for DifferenceSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "difference",
            "Difference",
            "Creates a set from the difference of 2 sets",
            "Utils/Set",
        );
        node.set_flowscript_name("set", "difference");
        node.set_receiver("set_in_1");

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("set_in_1", "Set 1", "Your First Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_input_pin(
            "set_in_2",
            "Set 2",
            "Your Second Set",
            VariableType::Generic,
        )
        .set_value_type(ValueType::HashSet);

        node.add_output_pin(
            "set_out",
            "Difference",
            "The difference set",
            VariableType::Generic,
        )
        .set_value_type(ValueType::HashSet);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let set_in_1: HashSet<Value> = context.evaluate_pin("set_in_1").await?;
        let set_in_2: HashSet<Value> = context.evaluate_pin("set_in_1").await?;
        let diff: Difference<Value, RandomState> = set_in_1.difference(&set_in_2);
        let result: HashSet<&Value> = HashSet::from_iter(diff);
        context.set_pin_value("set_out", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_out", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("set_in_1", board, Some(ValueType::HashSet), None);
        let _ = node.match_type("set_in_2", board, Some(ValueType::HashSet), None);
        node.harmonize_type(vec!["set_in_1", "set_out", "set_in_2"], true);
    }
}
