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
pub struct ArrayToSetNode {}

impl ArrayToSetNode {
    pub fn new() -> Self {
        ArrayToSetNode {}
    }
}

#[async_trait]
impl NodeLogic for ArrayToSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_to_set",
            "Array to Set",
            "Converts an array to a set",
            "Utils/Set",
        );
        node.set_flowscript_name("array", "toSet");
        node.set_receiver("array_in");

        node.add_icon("/flow/icons/ellipsis-vertical.svg");

        node.add_input_pin("array_in", "Array", "", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_output_pin("set_out", "Set", "", VariableType::Generic)
            .set_value_type(ValueType::HashSet);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array_in: HashSet<Value> = context.evaluate_pin("array_in").await?;
        let set_out: HashSet<Value> = array_in.into_iter().collect();

        context.set_pin_value("set_out", json!(set_out)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type(
            "array_in",
            board,
            Some(ValueType::Array),
            Some(ValueType::Array),
        );
        let _ = node.match_type(
            "set_out",
            board,
            Some(ValueType::HashSet),
            Some(ValueType::HashSet),
        );
        node.harmonize_type(vec!["array_in", "set_out"], true);
    }
}
