use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, geometry::GeometryKind, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::GeoCoordinate;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct Polygon {
    pub exterior: Vec<GeoCoordinate>,
    pub interiors: Vec<Vec<GeoCoordinate>>,
}

#[crate::register_node]
#[derive(Default)]
pub struct CellsToMultiPolygonNode {}

impl CellsToMultiPolygonNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CellsToMultiPolygonNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_cells_to_multi_polygon",
            "H3 Cells to Polygon",
            "Converts a set of H3 cells to polygon boundaries. Returns the outline(s) of the cell set, merging adjacent cells.",
            "Web/Geo/H3",
        );
        node.set_version(2);
        node.set_flowscript_name("h3", "cellsToMultiPolygon");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "cells",
            "Cells",
            "Array of H3 cell indices",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_default_value(Some(json!([])));

        crate::geo::pins::geometry_output(
            &mut node,
            "geometry_out",
            "Geometry",
            "Merged cell boundaries as a MultiPolygon, split at the antimeridian",
            Some(GeometryKind::MultiPolygon),
        );

        node.add_output_pin(
            "polygon_count",
            "Polygon Count",
            "Number of polygons in Geometry, including pieces split at the antimeridian",
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
        use h3o::{CellIndex, geom::SolventBuilder};
        use std::str::FromStr;

        crate::geo::pins::clear_output(context, "geometry_out").await?;
        let cell_strs: Vec<String> = context.evaluate_pin("cells").await?;

        let cells: Vec<CellIndex> = cell_strs
            .iter()
            .filter_map(|s| CellIndex::from_str(s).ok())
            .collect();

        if cells.is_empty() {
            context
                .set_pin_value(
                    "geometry_out",
                    json!({"type":"MultiPolygon","coordinates":[]}),
                )
                .await?;
            context.set_pin_value("polygon_count", json!(0)).await?;
            return Ok(());
        }

        let solvent = SolventBuilder::new().build();
        let multi_poly = solvent
            .dissolve(cells)
            .map_err(|e| flow_like_types::anyhow!("Failed to create polygon: {}", e))?;

        let geometry = crate::geo::geometry::integrations::h3_multipolygon_geometry(multi_poly)?;
        let polygon_count = geometry["coordinates"]
            .as_array()
            .expect("validated MultiPolygon coordinates")
            .len() as i64;

        context.set_pin_value("geometry_out", geometry).await?;
        context
            .set_pin_value("polygon_count", json!(polygon_count))
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
