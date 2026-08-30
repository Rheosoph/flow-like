use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct GridDiskNode {}

impl GridDiskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GridDiskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_grid_disk",
            "H3 Grid Disk",
            "Returns all H3 cells within k steps of the origin cell (a filled disk of hexagons). Useful for proximity searches and area coverage.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "gridDisk");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin("cell", "Cell", "Origin H3 cell index", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "k",
            "K (Radius)",
            "Number of rings around the origin (0 = just the origin cell)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_output_pin(
            "cells",
            "Cells",
            "Array of H3 cell indices in the disk",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of cells in the disk",
            VariableType::Integer,
        );

        node.set_long_running(false);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
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

        let cell_str: String = context.evaluate_pin("cell").await?;
        let k: i64 = context.evaluate_pin("k").await?;
        let k = k.clamp(0, 100) as u32;

        let cell = CellIndex::from_str(&cell_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index: {}", e))?;

        let disk: Vec<String> = cell
            .grid_disk::<Vec<_>>(k)
            .into_iter()
            .map(|c| c.to_string())
            .collect();

        let count = disk.len() as i64;

        context.set_pin_value("cells", json!(disk)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
