use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::collections::HashMap;

#[crate::register_node]
#[derive(Default)]
pub struct MakeMapNode {}

impl MakeMapNode {
    pub fn new() -> Self {
        MakeMapNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "make_map",
            "Make Map",
            "Creates an empty map (string keys)",
            "Utils/Map",
        );

        node.add_icon("/flow/icons/book-key.svg");

        node.add_output_pin("map_out", "Map", "The created map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let map_out: HashMap<String, flow_like_types::Value> = HashMap::new();
        context.set_pin_value("map_out", json!(map_out)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_out", board, Some(ValueType::HashMap), None);
    }
}
