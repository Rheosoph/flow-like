use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub enum AreaUnit {
    #[default]
    SquareMeters,
    SquareKilometers,
    SquareMiles,
    Hectares,
    Acres,
}

#[crate::register_node]
#[derive(Default)]
pub struct CellAreaNode {}

impl CellAreaNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CellAreaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_cell_area",
            "H3 Cell Area",
            "Calculates the area of an H3 cell in the specified unit.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "cellArea");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin("cell", "Cell", "H3 cell index", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "unit",
            "Unit",
            "Area unit for the result",
            VariableType::Struct,
        )
        .set_schema::<AreaUnit>()
        .set_default_value(Some(json!(AreaUnit::SquareMeters)));

        node.add_output_pin(
            "area",
            "Area",
            "Area of the cell in the specified unit",
            VariableType::Float,
        );

        node.add_output_pin(
            "resolution",
            "Resolution",
            "Resolution of the cell",
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

        let cell_str: String = context.evaluate_pin("cell").await?;
        let unit: AreaUnit = context.evaluate_pin("unit").await?;

        let cell = CellIndex::from_str(&cell_str)
            .map_err(|e| flow_like_types::anyhow!("Invalid H3 cell index: {}", e))?;

        let area_m2 = cell.area_m2();
        let area = match unit {
            AreaUnit::SquareMeters => area_m2,
            AreaUnit::SquareKilometers => area_m2 / 1_000_000.0,
            AreaUnit::SquareMiles => area_m2 / 2_589_988.11,
            AreaUnit::Hectares => area_m2 / 10_000.0,
            AreaUnit::Acres => area_m2 / 4_046.86,
        };

        let resolution = u8::from(cell.resolution()) as i64;

        context.set_pin_value("area", json!(area)).await?;
        context
            .set_pin_value("resolution", json!(resolution))
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
