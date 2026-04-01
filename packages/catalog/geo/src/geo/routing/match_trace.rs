use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::{
    GeoCoordinate,
    routing::osrm::{RouteProfile, RouteResult},
};

#[cfg(feature = "execute")]
use crate::geo::routing::osrm::{OsrmRoute, build_coordinate_string, map_osrm_routes};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct Tracepoint {
    pub name: String,
    pub distance: Option<f64>,
    pub coordinate: GeoCoordinate,
    pub hint: Option<String>,
    pub matchings_index: Option<usize>,
    pub waypoint_index: Option<usize>,
    pub alternatives_count: Option<usize>,
}

#[crate::register_node]
#[derive(Default)]
pub struct OsrmMatchTraceNode {}

impl OsrmMatchTraceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for OsrmMatchTraceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_osrm_match_trace",
            "OSRM Match Trace",
            "Snaps noisy GPS traces to the road network using OSRM map matching.",
            "Web/Geo/Routing",
        );
        node.add_icon("/flow/icons/route.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the trace matching request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "coordinates",
            "Coordinates",
            "Ordered GPS coordinates to match",
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
            VariableType::Struct,
        )
        .set_schema::<RouteProfile>()
        .set_default_value(Some(json!(RouteProfile::Car)));

        node.add_input_pin(
            "timestamps",
            "Timestamps",
            "Optional UNIX timestamps for each coordinate (seconds)",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "radiuses",
            "Radiuses",
            "Optional search radiuses in meters for each coordinate",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "gaps",
            "Gaps",
            "How to handle gaps: split or ignore",
            VariableType::String,
        )
        .set_default_value(Some(json!("split")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["split".to_string(), "ignore".to_string()])
                .build(),
        );

        node.add_input_pin(
            "tidy",
            "Tidy",
            "Simplify the matched geometry",
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
            "Triggered when map matching succeeds",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when map matching fails",
            VariableType::Execution,
        );

        node.add_output_pin(
            "matchings",
            "Matchings",
            "Matched routes for the trace",
            VariableType::Struct,
        )
        .set_schema::<RouteResult>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "primary_matching",
            "Primary Matching",
            "Primary matched route",
            VariableType::Struct,
        )
        .set_schema::<RouteResult>();

        node.add_output_pin(
            "tracepoints",
            "Tracepoints",
            "Tracepoints mapped to the road network",
            VariableType::Struct,
        )
        .set_schema::<Tracepoint>()
        .set_value_type(ValueType::Array);

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(6)
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
        let profile: RouteProfile = context.evaluate_pin("profile").await?;
        let timestamps: Vec<i64> = context.evaluate_pin("timestamps").await?;
        let radiuses: Vec<f64> = context.evaluate_pin("radiuses").await?;
        let gaps: String = context.evaluate_pin("gaps").await?;
        let tidy: bool = context.evaluate_pin("tidy").await?;
        let base_url: String = context.evaluate_pin("base_url").await?;

        if coordinates.is_empty() {
            return Err(flow_like_types::anyhow!(
                "At least one coordinate is required"
            ));
        }

        let coords_str = build_coordinate_string(&coordinates);
        let profile_str = profile.as_str();
        let base_url = base_url.trim_end_matches('/');

        let mut query_parts = vec![
            "overview=full".to_string(),
            "geometries=geojson".to_string(),
            "steps=true".to_string(),
            format!("gaps={}", gaps),
            format!("tidy={}", tidy),
        ];

        if !timestamps.is_empty() {
            let joined = timestamps
                .iter()
                .map(|ts| ts.to_string())
                .collect::<Vec<String>>()
                .join(";");
            query_parts.push(format!("timestamps={}", joined));
        }

        if !radiuses.is_empty() {
            let joined = radiuses
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<String>>()
                .join(";");
            query_parts.push(format!("radiuses={}", joined));
        }

        let url = format!(
            "{}/match/v1/{}/{}?{}",
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

        let body: OsrmMatchResponse = response.json().await?;
        if body.code != "Ok" {
            return Err(flow_like_types::anyhow!(
                "OSRM returned error: {}",
                body.message.unwrap_or_else(|| body.code.clone())
            ));
        }

        let matchings = map_osrm_routes(body.matchings.unwrap_or_default());
        let primary = matchings.first().cloned().unwrap_or_default();

        let tracepoints = body
            .tracepoints
            .unwrap_or_default()
            .into_iter()
            .map(|tp| {
                tp.map(|tp| Tracepoint {
                    name: tp.name,
                    distance: tp.distance,
                    coordinate: GeoCoordinate::new(tp.location[1], tp.location[0]),
                    hint: tp.hint,
                    matchings_index: tp.matchings_index,
                    waypoint_index: tp.waypoint_index,
                    alternatives_count: tp.alternatives_count,
                })
            })
            .collect::<Vec<Option<Tracepoint>>>();

        context.set_pin_value("matchings", json!(matchings)).await?;
        context
            .set_pin_value("primary_matching", json!(primary))
            .await?;
        context
            .set_pin_value("tracepoints", json!(tracepoints))
            .await?;

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
struct OsrmMatchResponse {
    code: String,
    message: Option<String>,
    tracepoints: Option<Vec<Option<OsrmTracepoint>>>,
    matchings: Option<Vec<OsrmRoute>>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct OsrmTracepoint {
    name: String,
    location: Vec<f64>,
    hint: Option<String>,
    distance: Option<f64>,
    matchings_index: Option<usize>,
    waypoint_index: Option<usize>,
    alternatives_count: Option<usize>,
}
