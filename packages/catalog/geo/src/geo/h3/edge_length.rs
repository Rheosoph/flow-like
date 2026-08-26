use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub enum LengthUnit {
    #[default]
    Meters,
    Kilometers,
    Miles,
    Feet,
}

#[crate::register_node]
#[derive(Default)]
pub struct EdgeLengthNode {}

impl EdgeLengthNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for EdgeLengthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "h3_edge_length",
            "H3 Edge Length",
            "Returns the average edge length of H3 cells at a given resolution.",
            "Web/Geo/H3",
        );
        node.set_flowscript_name("h3", "edgeLength");
        node.add_icon("/flow/icons/hexagon.svg");

        node.add_input_pin(
            "resolution",
            "Resolution",
            "H3 resolution (0-15)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(9)));

        node.add_input_pin(
            "unit",
            "Unit",
            "Length unit for the result",
            VariableType::Struct,
        )
        .set_schema::<LengthUnit>()
        .set_default_value(Some(json!(LengthUnit::Meters)));

        node.add_output_pin(
            "edge_length",
            "Edge Length",
            "Average edge length at this resolution",
            VariableType::Float,
        );

        node.add_output_pin(
            "cell_count",
            "Cell Count",
            "Total number of cells at this resolution covering Earth",
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
        use h3o::Resolution;

        let resolution: i64 = context.evaluate_pin("resolution").await?;
        let resolution = resolution.clamp(0, 15) as u8;
        let unit: LengthUnit = context.evaluate_pin("unit").await?;

        let res = Resolution::try_from(resolution)
            .map_err(|e| flow_like_types::anyhow!("Invalid resolution: {}", e))?;

        let edge_length_m = res.edge_length_m();
        let edge_length = match unit {
            LengthUnit::Meters => edge_length_m,
            LengthUnit::Kilometers => edge_length_m / 1_000.0,
            LengthUnit::Miles => edge_length_m / 1_609.344,
            LengthUnit::Feet => edge_length_m * 3.28084,
        };

        let cell_count = res.cell_count() as i64;

        context
            .set_pin_value("edge_length", json!(edge_length))
            .await?;
        context
            .set_pin_value("cell_count", json!(cell_count))
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
