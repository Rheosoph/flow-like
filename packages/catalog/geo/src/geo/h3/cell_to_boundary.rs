use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CellToBoundaryNode {}

impl CellToBoundaryNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CellToBoundaryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_cell_to_boundary",
            "H3 Cell Boundary",
            "Returns the polygon boundary (vertices) of an H3 cell. Useful for visualization and geospatial operations.",
            "Web/Geo/H3",
        );
        node.set_version(2);
        node.set_flowscript_name("h3", "cellToBoundary");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "cell",
            "Cell",
            "H3 cell index as a hexadecimal string",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        crate::geo::pins::geometry_output(
            &mut node,
            "geometry_out",
            "Geometry",
            "Cell boundary as a Polygon, or MultiPolygon when it crosses the antimeridian",
            None,
        );

        node.add_output_pin(
            "vertex_count",
            "Vertex Count",
            "Number of vertices (typically 6 for hexagons, 5 for pentagons)",
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

        crate::geo::pins::clear_output(context, "geometry_out").await?;
        let cell_str: String = context.evaluate_pin("cell").await?;

        let cell = CellIndex::from_str(&cell_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index: {}", e))?;

        let vertex_count = cell.boundary().len() as i64;

        context
            .set_pin_value(
                "geometry_out",
                crate::geo::geometry::integrations::h3_boundary_geometry(cell)?,
            )
            .await?;
        context
            .set_pin_value("vertex_count", json!(vertex_count))
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
