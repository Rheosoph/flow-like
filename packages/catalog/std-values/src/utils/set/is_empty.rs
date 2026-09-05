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
pub struct SetIsEmptyNode {}

impl SetIsEmptyNode {
    pub fn new() -> Self {
        SetIsEmptyNode {}
    }
}

#[async_trait]
impl NodeLogic for SetIsEmptyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_is_empty",
            "Is Empty",
            "Checks if a hash set is empty or not",
            "Utils/Set",
        );
        node.set_flowscript_name("set", "isEmpty");
        node.set_receiver("set_in");

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("set_in", "Set", "Your Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_output_pin(
            "is_empty",
            "Is Empty?",
            "Does it have any values or not?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let set_in: &HashSet<Value> = &context.evaluate_pin("set_in").await?;
        context
            .set_pin_value("is_empty", json!(set_in.is_empty()))
            .await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_in", board, Some(ValueType::HashSet), None);
        node.harmonize_type(vec!["set_in"], true);
    }
}
