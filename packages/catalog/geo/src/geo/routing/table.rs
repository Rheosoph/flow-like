use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::GeoCoordinate;

#[cfg(feature = "execute")]
use crate::geo::routing::osrm::build_coordinate_string;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct TableMatrixRow {
    pub values: Vec<Option<f64>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct TableResult {
    pub durations: Vec<TableMatrixRow>,
    pub distances: Vec<TableMatrixRow>,
}

#[crate::register_node]
#[derive(Default)]
pub struct OsrmTableNode {}

impl OsrmTableNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for OsrmTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_osrm_table",
            "OSRM Table",
            "Computes travel time and distance matrices between coordinates using OSRM.",
            "Web/Geo/Routing",
        );
        node.add_icon("/flow/icons/table.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the matrix request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "coordinates",
            "Coordinates",
            "List of coordinates to include in the matrix",
            VariableType::Struct,
        )
        .set_schema::<GeoCoordinate>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "profile",
            "Profile",
            "Transportation mode: Car, Bike, or Foot",
            VariableType::String,
        )
        .set_default_value(Some(json!("Car")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Car".to_string(),
                    "Bike".to_string(),
                    "Foot".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "sources",
            "Sources",
            "Optional indices of source coordinates",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "destinations",
            "Destinations",
            "Optional indices of destination coordinates",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "include_durations",
            "Include Durations",
            "Return travel time matrix",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_distances",
            "Include Distances",
            "Return travel distance matrix",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "base_url",
            "Base URL",
            "OSRM server base URL",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://router.project-osrm.org")));

        node.add_output_pin(
            "exec_success",
            "Success",
            "Triggered when the matrix request succeeds",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when the matrix request fails",
            VariableType::Execution,
        );

        node.add_output_pin(
            "durations",
            "Durations",
            "Matrix of travel times in seconds",
            VariableType::Struct,
        )
        .set_schema::<TableMatrixRow>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "distances",
            "Distances",
            "Matrix of travel distances in meters",
            VariableType::Struct,
        )
        .set_schema::<TableMatrixRow>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "result",
            "Result",
            "Matrix result containing durations and distances",
            VariableType::Struct,
        )
        .set_schema::<TableResult>();

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::reqwest;

        context.deactivate_exec_pin("exec_success").await?;
        context.activate_exec_pin("exec_error").await?;

        let coordinates: Vec<GeoCoordinate> = context.evaluate_pin("coordinates").await?;
        let profile: String = context.evaluate_pin("profile").await?;
        let sources: Vec<i64> = context.evaluate_pin("sources").await?;
        let destinations: Vec<i64> = context.evaluate_pin("destinations").await?;
        let include_durations: bool = context.evaluate_pin("include_durations").await?;
        let include_distances: bool = context.evaluate_pin("include_distances").await?;
        let base_url: String = context.evaluate_pin("base_url").await?;

        if coordinates.is_empty() {
            return Err(flow_like_types::anyhow!(
                "At least one coordinate is required"
            ));
        }

        let coords_str = build_coordinate_string(&coordinates);
        let profile_str = match profile.as_str() {
            "Car" | "car" => "driving",
            "Bike" | "bike" => "cycling",
            "Foot" | "foot" => "foot",
            _ => return Err(flow_like_types::anyhow!("Unsupported profile: {}", profile)),
        };
        let base_url = base_url.trim_end_matches('/');

        let annotations = match (include_durations, include_distances) {
            (true, true) => "duration,distance",
            (true, false) => "duration",
            (false, true) => "distance",
            (false, false) => "duration",
        };

        let mut query_parts = vec![format!("annotations={}", annotations)];

        if !sources.is_empty() {
            let joined = sources
                .iter()
                .map(|idx| idx.to_string())
                .collect::<Vec<String>>()
                .join(";");
            query_parts.push(format!("sources={}", joined));
        }

        if !destinations.is_empty() {
            let joined = destinations
                .iter()
                .map(|idx| idx.to_string())
                .collect::<Vec<String>>()
                .join(";");
            query_parts.push(format!("destinations={}", joined));
        }

        let url = format!(
            "{}/table/v1/{}/{}?{}",
            base_url,
            profile_str,
            coords_str,
            query_parts.join("&")
        );

        let client = reqwest::Client::builder()
            .user_agent("FlowLike/1.0")
            .build()?;

        let response = client.get(&url).send().await?;
        if !response.status().is_success() {
            return Err(flow_like_types::anyhow!(
                "OSRM API returned status: {}",
                response.status()
            ));
        }

        let body: OsrmTableResponse = response.json().await?;
        if body.code != "Ok" {
            return Err(flow_like_types::anyhow!(
                "OSRM returned error: {}",
                body.message.unwrap_or_else(|| body.code.clone())
            ));
        }

        let durations = body
            .durations
            .unwrap_or_default()
            .into_iter()
            .map(|values| TableMatrixRow { values })
            .collect::<Vec<TableMatrixRow>>();
        let distances = body
            .distances
            .unwrap_or_default()
            .into_iter()
            .map(|values| TableMatrixRow { values })
            .collect::<Vec<TableMatrixRow>>();
        let result = TableResult {
            durations: durations.clone(),
            distances: distances.clone(),
        };

        context.set_pin_value("durations", json!(durations)).await?;
        context.set_pin_value("distances", json!(distances)).await?;
        context.set_pin_value("result", json!(result)).await?;

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("exec_success").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct OsrmTableResponse {
    code: String,
    message: Option<String>,
    durations: Option<Vec<Vec<Option<f64>>>>,
    distances: Option<Vec<Vec<Option<f64>>>>,
}
