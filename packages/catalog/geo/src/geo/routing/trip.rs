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
pub struct TripWaypoint {
    pub name: String,
    pub distance: f64,
    pub coordinate: GeoCoordinate,
    pub hint: Option<String>,
    pub waypoint_index: Option<usize>,
}

#[crate::register_node]
#[derive(Default)]
pub struct OsrmTripNode {}

impl OsrmTripNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for OsrmTripNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_osrm_trip",
            "OSRM Trip",
            "Plans the shortest round trip through multiple coordinates using OSRM.",
            "Web/Geo/Routing",
        );
        node.set_flowscript_name("geo", "osrmTrip");
        node.add_icon("/flow/icons/route.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the trip planning request",
            VariableType::Execution,
        );
        node.add_input_pin(
            "coordinates",
            "Coordinates",
            "Ordered coordinates for the trip",
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
            "roundtrip",
            "Roundtrip",
            "Return to the starting point",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "source",
            "Source",
            "Source location: any, first, or last",
            VariableType::String,
        )
        .set_default_value(Some(json!("any")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "any".to_string(),
                    "first".to_string(),
                    "last".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "destination",
            "Destination",
            "Destination location: any, first, or last",
            VariableType::String,
        )
        .set_default_value(Some(json!("any")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "any".to_string(),
                    "first".to_string(),
                    "last".to_string(),
                ])
                .build(),
        );

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
            "Triggered when trip planning succeeds",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when trip planning fails",
            VariableType::Execution,
        );

        node.add_output_pin("trip", "Trip", "Primary trip result", VariableType::Struct)
            .set_schema::<RouteResult>();

        node.add_output_pin(
            "trips",
            "Trips",
            "All trip results returned by OSRM",
            VariableType::Struct,
        )
        .set_schema::<RouteResult>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "waypoints",
            "Waypoints",
            "Optimized trip waypoints",
            VariableType::Struct,
        )
        .set_schema::<TripWaypoint>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "distance",
            "Distance",
            "Total trip distance in meters",
            VariableType::Float,
        );

        node.add_output_pin(
            "duration",
            "Duration",
            "Total trip duration in seconds",
            VariableType::Float,
        );

        node.add_output_pin(
            "geometry",
            "Geometry",
            "Trip geometry as array of coordinates",
            VariableType::Struct,
        )
        .set_schema::<GeoCoordinate>()
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
        let roundtrip: bool = context.evaluate_pin("roundtrip").await?;
        let source: String = context.evaluate_pin("source").await?;
        let destination: String = context.evaluate_pin("destination").await?;
        let base_url: String = context.evaluate_pin("base_url").await?;

        if coordinates.len() < 2 {
            return Err(flow_like_types::anyhow!(
                "At least two coordinates are required"
            ));
        }

        let coords_str = build_coordinate_string(&coordinates);
        let profile_str = profile.as_str();
        let base_url = base_url.trim_end_matches('/');

        let query_parts = [
            "overview=full".to_string(),
            "geometries=geojson".to_string(),
            "steps=true".to_string(),
            format!("roundtrip={}", roundtrip),
            format!("source={}", source),
            format!("destination={}", destination),
        ];

        let url = format!(
            "{}/trip/v1/{}/{}?{}",
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

        let body: OsrmTripResponse = response.json().await?;
        if body.code != "Ok" {
            return Err(flow_like_types::anyhow!(
                "OSRM returned error: {}",
                body.message.unwrap_or_else(|| body.code.clone())
            ));
        }

        let trips = map_osrm_routes(body.trips.unwrap_or_default());
        let primary = trips.first().cloned().unwrap_or_default();
        let waypoints = body
            .waypoints
            .unwrap_or_default()
            .into_iter()
            .map(|wp| TripWaypoint {
                name: wp.name,
                distance: wp.distance.unwrap_or_default(),
                coordinate: GeoCoordinate::new(wp.location[1], wp.location[0]),
                hint: wp.hint,
                waypoint_index: wp.waypoint_index,
            })
            .collect::<Vec<TripWaypoint>>();

        context.set_pin_value("trip", json!(primary)).await?;
        context.set_pin_value("trips", json!(trips)).await?;
        context.set_pin_value("waypoints", json!(waypoints)).await?;
        context
            .set_pin_value("distance", json!(primary.distance))
            .await?;
        context
            .set_pin_value("duration", json!(primary.duration))
            .await?;
        context
            .set_pin_value("geometry", json!(primary.geometry))
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
struct OsrmTripResponse {
    code: String,
    message: Option<String>,
    waypoints: Option<Vec<OsrmWaypoint>>,
    trips: Option<Vec<OsrmRoute>>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct OsrmWaypoint {
    name: String,
    location: Vec<f64>,
    distance: Option<f64>,
    hint: Option<String>,
    waypoint_index: Option<usize>,
}
