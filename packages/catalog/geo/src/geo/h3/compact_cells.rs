use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CompactCellsNode {}

impl CompactCellsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CompactCellsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_compact_cells",
            "H3 Compact Cells",
            "Compacts a set of H3 cells by replacing groups of cells with their parent when all children are present. Reduces the number of cells while covering the same area.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "compactCells");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "cells",
            "Cells",
            "Array of H3 cell indices to compact",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "compacted",
            "Compacted",
            "Array of compacted H3 cell indices (may contain mixed resolutions)",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "original_count",
            "Original Count",
            "Number of input cells",
            VariableType::Integer,
        );

        node.add_output_pin(
            "compacted_count",
            "Compacted Count",
            "Number of cells after compaction",
            VariableType::Integer,
        );

        node.set_long_running(false);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use h3o::CellIndex;
        use std::str::FromStr;

        let cell_strs: Vec<String> = context.evaluate_pin("cells").await?;
        let original_count = cell_strs.len() as i64;

        let mut cells: Vec<CellIndex> = cell_strs
            .iter()
            .filter_map(|s| CellIndex::from_str(s).ok())
            .collect();

        CellIndex::compact(&mut cells)
            .map_err(|e| flow_like_types::anyhow!("Failed to compact cells: {}", e))?;

        let compacted: Vec<String> = cells.iter().map(|c| c.to_string()).collect();
        let compacted_count = compacted.len() as i64;

        context.set_pin_value("compacted", json!(compacted)).await?;
        context
            .set_pin_value("original_count", json!(original_count))
            .await?;
        context
            .set_pin_value("compacted_count", json!(compacted_count))
            .await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
