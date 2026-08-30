use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct ClearMapNode {}

impl ClearMapNode {
    pub fn new() -> Self {
        ClearMapNode {}
    }
}

#[async_trait]
impl NodeLogic for ClearMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_clear",
            "Clear Map",
            "Removes all entries from a map",
            "Utils/Map",
        );
        node.set_flowscript_name("map", "clear");
        node.set_receiver("map_in");

        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin("map_out", "Emptied", "Empty Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let map_out: flow_like_types::json::Map<String, Value> = flow_like_types::json::Map::new();

        context
            .set_pin_value("map_out", Value::Object(map_out))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_in", board, Some(ValueType::HashMap), None);
        let _ = node.match_type("map_out", board, Some(ValueType::HashMap), None);
        node.harmonize_type(vec!["map_in", "map_out"], true);
    }
}
