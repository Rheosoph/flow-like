use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SizeMapNode {}

impl SizeMapNode {
    pub fn new() -> Self {
        SizeMapNode {}
    }
}

#[async_trait]
impl NodeLogic for SizeMapNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "map_size",
            "Size",
            "Gets the number of entries in the map",
            "Utils/Map",
        );
        node.set_flowscript_name("map", "size");
        node.set_receiver("map_in");

        node.add_icon("/flow/icons/book-key.svg");

        node.add_input_pin("map_in", "Map", "Your Map", VariableType::Generic)
            .set_value_type(ValueType::HashMap);

        node.add_output_pin(
            "size",
            "Size",
            "Number of entries in the map",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let map_in = context.evaluate_pin_to_ref("map_in").await?;

        let size = map_in.as_object().map(|obj| obj.len()).unwrap_or(0);

        context.set_pin_value("size", json!(size)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("map_in", board, Some(ValueType::HashMap), None);
    }
}
