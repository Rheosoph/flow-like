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
pub struct FilterArrayFieldNode {}

impl FilterArrayFieldNode {
    pub fn new() -> Self {
        FilterArrayFieldNode {}
    }
}

fn remove_field_from_value(value: &mut Value, field: &str) -> bool {
    if let Some(obj) = value.as_object_mut() {
        obj.remove(field).is_some()
    } else {
        false
    }
}

#[async_trait]
impl NodeLogic for FilterArrayFieldNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_filter_field",
            "Filter Array Field",
            "Removes a specific field from every struct in an array. Elements without the field are kept unchanged. Returns the filtered array and count of removed fields.",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin(
            "array_in",
            "Array",
            "Array of structs to filter",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "field",
            "Field",
            "Field name to remove from each struct",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Array with the field removed from each struct",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "removed_count",
            "Removed Count",
            "Number of fields that were removed",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let field: String = context.evaluate_pin("field").await?;

        let mut removed_count = 0i64;

        for item in &mut array_in {
            if remove_field_from_value(item, &field) {
                removed_count += 1;
            }
        }

        context.set_pin_value("array_out", json!(array_in)).await?;
        context
            .set_pin_value("removed_count", json!(removed_count))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
    }
}
