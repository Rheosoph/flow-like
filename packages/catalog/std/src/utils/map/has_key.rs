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
pub struct HasKeyMapNode {}

impl HasKeyMapNode {
    pub fn new() -> Self {
        HasKeyMapNode {}
    }
}

#[async_trait]
impl NodeLogic for HasKeyMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_has_key",
            "Has Key",
            "Checks if a key exists in the map",
            "Utils/Map",
        );

        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_input_pin("key", "Key", "Key to check", VariableType::String);

        node.add_output_pin(
            "has_key",
            "Has Key",
            "Does the map contain the key?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let map_in = context.evaluate_pin_to_ref("map_in").await?;
        let key: String = context.evaluate_pin("key").await?;

        let has_key = map_in
            .as_object()
            .map(|obj| obj.contains_key(&key))
            .unwrap_or(false);

        context.set_pin_value("has_key", json!(has_key)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_in", board, Some(ValueType::HashMap), None);
    }
}
