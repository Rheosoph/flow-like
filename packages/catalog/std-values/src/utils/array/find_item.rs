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
pub struct FindItemInArrayNode {}

impl FindItemInArrayNode {
    pub fn new() -> Self {
        FindItemInArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for FindItemInArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_find_item",
            "Find Item",
            "Finds the index of an item in an array",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "indexOf");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_input_pin("item", "Item", "Item to find", VariableType::Generic);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        node.add_output_pin(
            "index",
            "Index",
            "Index of the item (-1 if not found)",
            VariableType::Integer,
        );
        node.add_output_pin(
            "found",
            "Found",
            "Was the item found?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let array_in = context.evaluate_pin_to_ref("array_in").await?;
        let item: Value = context.evaluate_pin("item").await?;
        let mut found = false;

        let mut index = -1;

        if let Some(array) = array_in.as_array() {
            for (i, value) in array.iter().enumerate() {
                if value == &item {
                    found = true;
                    index = i as i32;
                    break;
                }
            }
        }

        context.set_pin_value("index", json!(index)).await?;
        context.set_pin_value("found", json!(found)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("item", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["array_in", "item"], true);
    }
}
