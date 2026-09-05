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
pub struct RemoveMapNode {}

impl RemoveMapNode {
    pub fn new() -> Self {
        RemoveMapNode {}
    }
}

#[async_trait]
impl NodeLogic for RemoveMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_remove",
            "Remove Key",
            "Removes a key from the map",
            "Utils/Map",
        );
        node.set_flowscript_name("map", "remove");
        node.set_receiver("map_in");
        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_input_pin("key", "Key", "Key to remove", VariableType::String);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin("map_out", "Map", "Adjusted Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_output_pin(
            "value",
            "Value",
            "The removed value (null if key not found)",
            VariableType::Generic,
        );

        node.add_output_pin(
            "was_present",
            "Was Present",
            "Was the key in the map?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let map_in: Value = context.evaluate_pin("map_in").await?;
        let key: String = context.evaluate_pin("key").await?;

        let mut map_out = match map_in {
            Value::Object(m) => m,
            _ => flow_like_types::json::Map::new(),
        };

        let removed_value = map_out.remove(&key);
        let was_present = removed_value.is_some();

        context
            .set_pin_value("map_out", Value::Object(map_out))
            .await?;
        context
            .set_pin_value("value", removed_value.unwrap_or(Value::Null))
            .await?;
        context
            .set_pin_value("was_present", json!(was_present))
            .await?;
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
