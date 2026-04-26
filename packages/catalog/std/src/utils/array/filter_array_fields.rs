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
pub struct FilterArrayFieldsNode {}

impl FilterArrayFieldsNode {
    pub fn new() -> Self {
        FilterArrayFieldsNode {}
    }
}

fn remove_fields_from_value(value: &mut Value, fields: &[String]) -> i64 {
    let mut count = 0i64;
    if let Some(obj) = value.as_object_mut() {
        for field in fields {
            if obj.remove(field).is_some() {
                count += 1;
            }
        }
    }
    count
}

#[async_trait]
impl NodeLogic for FilterArrayFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_filter_fields",
            "Filter Array Fields",
            "Removes multiple fields from every struct in an array. Elements without the fields are kept unchanged. Returns the filtered array and count of removed fields.",
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
            "fields",
            "Fields",
            "Array of field names to remove from each struct",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Array with the fields removed from each struct",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "removed_count",
            "Removed Count",
            "Total number of fields that were removed",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let fields: Vec<String> = context.evaluate_pin("fields").await?;

        let mut removed_count = 0i64;

        for item in &mut array_in {
            removed_count += remove_fields_from_value(item, &fields);
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
