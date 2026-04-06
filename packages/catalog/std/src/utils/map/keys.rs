use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct KeysMapNode {}

impl KeysMapNode {
    pub fn new() -> Self {
        KeysMapNode {}
    }
}

#[async_trait]
impl NodeLogic for KeysMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_keys",
            "Keys",
            "Gets all keys from the map as an array",
            "Utils/Map",
        );

        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_output_pin("keys", "Keys", "Array of all keys", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let map_in = context.evaluate_pin_to_ref("map_in").await?;

        let keys: Vec<String> = map_in
            .as_object()
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default();

        context.set_pin_value("keys", json!(keys)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_in", board, Some(ValueType::HashMap), None);
        let _ = node.match_type("keys", board, Some(ValueType::Array), None);
    }
}
