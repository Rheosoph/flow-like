use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use crate::geo::GeoCoordinate;

#[crate::register_node]
#[derive(Default)]
pub struct LatLngToCellNode {}

impl LatLngToCellNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LatLngToCellNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_latlng_to_cell",
            "Lat/Lng to H3 Cell",
            "Converts a geographic coordinate to an H3 cell index at the specified resolution. H3 is a hierarchical hexagonal grid system.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "latlngToCell");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "coordinate",
            "Coordinate",
            "The geographic coordinate (latitude, longitude)",
            VariableType::Struct,
        )
        .set_schema::<GeoCoordinate>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "resolution",
            "Resolution",
            "H3 resolution (0-15). Higher = smaller cells. 0 = ~4,357,449 km², 15 = ~0.9 m²",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(9)));

        node.add_output_pin(
            "cell",
            "Cell",
            "H3 cell index as a hexadecimal string",
            VariableType::String,
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
        use h3o::{CellIndex, LatLng, Resolution};

        let coordinate: GeoCoordinate = context.evaluate_pin("coordinate").await?;
        let resolution: i64 = context.evaluate_pin("resolution").await?;
        let resolution = resolution.clamp(0, 15) as u8;

        let latlng = LatLng::new(coordinate.latitude, coordinate.longitude)
            .map_err(|e| flow_like_types::anyhow!("Invalid coordinate: {}", e))?;

        let res = Resolution::try_from(resolution)
            .map_err(|e| flow_like_types::anyhow!("Invalid resolution: {}", e))?;

        let cell: CellIndex = latlng.to_cell(res);
        let cell_str = cell.to_string();

        context.set_pin_value("cell", json!(cell_str)).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
