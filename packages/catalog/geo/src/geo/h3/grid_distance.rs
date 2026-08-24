use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct GridDistanceNode {}

impl GridDistanceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GridDistanceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_grid_distance",
            "H3 Grid Distance",
            "Calculates the grid distance (number of steps) between two H3 cells. Both cells must be at the same resolution.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "gridDistance");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "cell_a",
            "Cell A",
            "First H3 cell index",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "cell_b",
            "Cell B",
            "Second H3 cell index",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "distance",
            "Distance",
            "Grid distance (number of hexagon steps) between the cells",
            VariableType::Integer,
        );

        node.set_long_running(false);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
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
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index A: {}", e))?;
        let cell_b = CellIndex::from_str(&cell_b_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index B: {}", e))?;

        let distance = cell_a
            .grid_distance(cell_b)
            .map_err(|e| flow_like_types::anyhow!("Cannot compute grid distance: {}", e))?;

        context
            .set_pin_value("distance", json!(distance as i64))
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
