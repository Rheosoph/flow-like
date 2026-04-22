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
pub struct SetMapNode {}

impl SetMapNode {
    pub fn new() -> Self {
        SetMapNode {}
    }
}

#[async_trait]
impl NodeLogic for SetMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_set",
            "Set Value",
            "Sets a value in a map at the given key",
            "Utils/Map",
        );
        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_input_pin("key", "Key", "Key to set", VariableType::String);

        node.add_input_pin("value", "Value", "Value to set", VariableType::Generic);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin("map_out", "Map", "Adjusted Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_output_pin(
            "replaced",
            "Replaced",
            "Was an existing value replaced?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let map_in: Value = context.evaluate_pin("map_in").await?;
        let key: String = context.evaluate_pin("key").await?;
        let value: Value = context.evaluate_pin("value").await?;

        let mut map_out = match map_in {
            Value::Object(m) => m,
            _ => flow_like_types::json::Map::new(),
        };

        let replaced = map_out.contains_key(&key);
        map_out.insert(key, value);

        context
            .set_pin_value("map_out", Value::Object(map_out))
            .await?;
        context.set_pin_value("replaced", json!(replaced)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_out", board, Some(ValueType::HashMap), None);
        let _ = node.match_type("map_in", board, Some(ValueType::HashMap), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["map_in", "map_out", "value"], true);
    }
}
