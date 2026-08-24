use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct GridPathNode {}

impl GridPathNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GridPathNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_grid_path",
            "H3 Grid Path",
            "Finds a path of H3 cells between two cells. Returns all cells along the shortest path. Both cells must be at the same resolution.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "gridPath");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "cell_a",
            "Start Cell",
            "Starting H3 cell index",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "cell_b",
            "End Cell",
            "Ending H3 cell index",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "path",
            "Path",
            "Array of H3 cell indices along the path (including start and end)",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "length",
            "Length",
            "Number of cells in the path",
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

        let cell_a_str: String = context.evaluate_pin("cell_a").await?;
        let cell_b_str: String = context.evaluate_pin("cell_b").await?;

        let cell_a = CellIndex::from_str(&cell_a_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 start cell index: {}", e))?;
        let cell_b = CellIndex::from_str(&cell_b_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 end cell index: {}", e))?;

        let path_result: Result<Vec<CellIndex>, _> = cell_a
            .grid_path_cells(cell_b)
            .map_err(|e| flow_like_types::anyhow!("Cannot compute grid path: {}", e))?
            .collect();

        let path_cells = path_result
            .map_err(|e| flow_like_types::anyhow!("Error in path computation: {}", e))?;

        let path: Vec<String> = path_cells.iter().map(|c| c.to_string()).collect();
        let length = path.len() as i64;

        context.set_pin_value("path", json!(path)).await?;
        context.set_pin_value("length", json!(length)).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
