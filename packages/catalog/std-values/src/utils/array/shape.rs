use super::sort::{generic_array_pin, harmonized_element_type};
use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ChunkArrayNode {}

impl ChunkArrayNode {
    pub fn new() -> Self {
        ChunkArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for ChunkArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_chunk",
            "Chunk",
            "Splits an array into batches of a fixed size",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "chunk");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/grip.svg");
        node.set_scores(pure_scores());

        generic_array_pin(&mut node, "array_in", "Array", "Your Array", false);
        node.add_input_pin("size", "Size", "Elements per batch", VariableType::Integer)
            .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "chunks",
            "Chunks",
            "One entry per batch, each holding up to Size elements",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );
        node.add_output_pin(
            "chunk_count",
            "Chunk Count",
            "How many batches were produced",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let size: i64 = context.evaluate_pin("size").await?;

        if size <= 0 {
            return Err(flow_like_types::anyhow!(
                "Chunk size must be at least 1, got {size}"
            ));
        }

        let chunks: Vec<Value> = array
            .chunks(size as usize)
            .map(|chunk| json!(chunk.to_vec()))
            .collect();

        context
            .set_pin_value("chunk_count", json!(chunks.len() as i64))
            .await?;
        context.set_pin_value("chunks", json!(chunks)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FlattenArrayNode {}

impl FlattenArrayNode {
    pub fn new() -> Self {
        FlattenArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for FlattenArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_flatten",
            "Flatten",
            "Pulls nested arrays up into a single array",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "flatten");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/grip.svg");
        node.set_scores(pure_scores());

        generic_array_pin(&mut node, "array_in", "Array", "Your Array", false);
        node.add_input_pin(
            "depth",
            "Depth",
            "How many levels to flatten, -1 for all of them",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        generic_array_pin(&mut node, "array_out", "Array", "The flattened array", true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array: Vec<Value> = context.evaluate_pin("array_in").await?;
        let depth: i64 = context.evaluate_pin("depth").await?;

        fn flatten(values: Vec<Value>, depth: i64) -> Vec<Value> {
            if depth == 0 {
                return values;
            }
            values
                .into_iter()
                .flat_map(|value| match value {
                    Value::Array(nested) => flatten(nested, depth - 1),
                    other => vec![other],
                })
                .collect()
        }

        let flattened = flatten(array, if depth < 0 { i64::MAX } else { depth });
        context.set_pin_value("array_out", json!(flattened)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = harmonized_element_type(node, board).await;
    }
}
