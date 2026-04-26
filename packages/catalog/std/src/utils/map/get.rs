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
pub struct GetMapNode {}

impl GetMapNode {
    pub fn new() -> Self {
        GetMapNode {}
    }
}

#[async_trait]
impl NodeLogic for GetMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_get",
            "Get Value",
            "Gets a value from a map by key",
            "Utils/Map",
        );

        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_input_pin("key", "Key", "Key to get", VariableType::String);

        node.add_output_pin(
            "value",
            "Value",
            "Value at the specified key",
            VariableType::Generic,
        );

        node.add_output_pin(
            "found",
            "Found",
            "Was the key found in the map?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let map_in = context.evaluate_pin_to_ref("map_in").await?;
        let key: String = context.evaluate_pin("key").await?;

        let mut found = false;
        let mut value = Value::Null;

        if let Some(obj) = map_in.as_object()
            && let Some(v) = obj.get(&key)
        {
            value = v.clone();
            found = true;
        }

        context.set_pin_value("value", json!(value)).await?;
        context.set_pin_value("found", json!(found)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_in", board, Some(ValueType::HashMap), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["map_in", "value"], true);
    }
}
