use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail, json::json};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct BatchSetArrayNode {}

impl BatchSetArrayNode {
    pub fn new() -> Self {
        BatchSetArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for BatchSetArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_batch_set",
            "Batch Set",
            "Set multiple elements at specific indices in one operation. More efficient than multiple single sets.",
            "Utils/Array/Batch",
        );
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );

        node.add_input_pin(
            "indices",
            "Indices",
            "Array of indices to set",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "values",
            "Values",
            "Array of values to set (must match indices length)",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Array with all values set",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let indices: Vec<usize> = context.evaluate_pin("indices").await?;
        let values: Vec<Value> = context.evaluate_pin("values").await?;

        if indices.len() != values.len() {
            bail!(
                "Indices and values must have the same length (got {} indices, {} values)",
                indices.len(),
                values.len()
            );
        }

        for (idx, value) in indices.into_iter().zip(values) {
            if idx >= array_in.len() {
                bail!(
                    "Index {} out of bounds (array length: {})",
                    idx,
                    array_in.len()
                );
            }
            array_in[idx] = value;
        }

        context.set_pin_value("array_out", json!(array_in)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("values", board, Some(ValueType::Array), None);
        node.harmonize_type(vec!["array_in", "array_out", "values"], true);
    }
}
