use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::geo::GeoCoordinate;

#[crate::register_node]
#[derive(Default)]
pub struct CellToLatLngNode {}

impl CellToLatLngNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CellToLatLngNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_cell_to_latlng",
            "H3 Cell to Lat/Lng",
            "Converts an H3 cell index to the geographic coordinate of its center point.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "cellToLatlng");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "cell",
            "Cell",
            "H3 cell index as a hexadecimal string",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "coordinate",
            "Coordinate",
            "The center coordinate of the H3 cell",
            VariableType::Struct,
        )
        .set_schema::<GeoCoordinate>();

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
        use h3o::{CellIndex, LatLng};
        use std::str::FromStr;

        let cell_str: String = context.evaluate_pin("cell").await?;

        let cell = CellIndex::from_str(&cell_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index: {}", e))?;

        let latlng = LatLng::from(cell);
        let coordinate = GeoCoordinate::new(latlng.lat(), latlng.lng());

        context
            .set_pin_value("coordinate", json!(coordinate))
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
