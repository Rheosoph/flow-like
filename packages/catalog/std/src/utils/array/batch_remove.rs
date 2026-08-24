use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BatchRemoveArrayNode {}

impl BatchRemoveArrayNode {
    pub fn new() -> Self {
        BatchRemoveArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for BatchRemoveArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_batch_remove",
            "Batch Remove",
            "Remove multiple elements at specific indices in one operation. More efficient than multiple single removes. Indices are processed in descending order to maintain correctness.",
            "Utils/Array/Batch",
        );
        node.set_flowscript_name("array", "batchRemove");
        node.set_receiver("array_in");
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
            "Array of indices to remove",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Array with elements removed",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node.add_output_pin(
            "removed",
            "Removed",
            "Array of removed values",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let mut indices: Vec<usize> = context.evaluate_pin("indices").await?;

        for &idx in &indices {
            if idx >= array_in.len() {
                bail!(
                    "Index {} out of bounds (array length: {})",
                    idx,
                    array_in.len()
                );
            }
        }

        // Sort indices in descending order to remove from end first
        indices.sort_unstable_by(|a, b| b.cmp(a));
        indices.dedup();

        let mut removed = Vec::with_capacity(indices.len());
        for idx in indices {
            removed.push(array_in.remove(idx));
        }

        // Reverse to match original index order
        removed.reverse();

        context.set_pin_value("array_out", json!(array_in)).await?;
        context.set_pin_value("removed", json!(removed)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("removed", board, Some(ValueType::Array), None);
        node.harmonize_type(vec!["array_in", "array_out", "removed"], true);
    }
}
