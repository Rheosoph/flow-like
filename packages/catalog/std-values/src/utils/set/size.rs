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
pub struct SetGetSizeNode {}

impl SetGetSizeNode {
    pub fn new() -> Self {
        SetGetSizeNode {}
    }
}

#[async_trait]
impl NodeLogic for SetGetSizeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_get_size",
            "Get Size",
            "Gets the size of the hash set (how many elements)",
            "Utils/Set",
        );
        node.set_flowscript_name("set", "size");
        node.set_receiver("set_in");

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("set_in", "Set", "Your Set", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node.add_output_pin(
            "size",
            "Size",
            "How many elements does it have",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let set_in: &HashSet<Value> = &context.evaluate_pin("set_in").await?;
        context.set_pin_value("size", json!(set_in.len())).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("set_in", board, Some(ValueType::HashSet), None);
        node.harmonize_type(vec!["set_in"], true);
    }
}
