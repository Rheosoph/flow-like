use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, geometry::GeometryKind, json::json};

use crate::geo::pins::{geometry_input, geometry_output};
#[cfg(feature = "execute")]
use serde::Deserialize;

use crate::geo::routing::osrm::RouteResult;

#[cfg(feature = "execute")]
use crate::geo::routing::osrm::{OsrmRoute, build_coordinate_string, map_osrm_routes};

#[crate::register_node]
#[derive(Default)]
pub struct PlanRouteNode {}

impl PlanRouteNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for PlanRouteNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_plan_route",
            "Plan Route",
            "Plans a route between two points using the OSRM routing service. Returns turn-by-turn directions, distance, and duration.",
            "Web/Geo/Routing",
        );
        node.set_version(2);
        node.set_flowscript_name("geo", "planRoute");
        node.add_icon("/flow/icons/map.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate route planning",
            VariableType::Execution,
        );

        geometry_input(
            &mut node,
            "start_geometry",
            "Start",
            "Starting Point geometry for the route",
            Some(GeometryKind::Point),
        );
        geometry_input(
            &mut node,
            "end_geometry",
            "End",
            "Ending Point geometry for the route",
            Some(GeometryKind::Point),
        );
        geometry_input(
            &mut node,
            "waypoint_geometries",
            "Waypoint Geometries",
            "Optional intermediate Point geometries in visit order",
            Some(GeometryKind::Point),
        )
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_optional(true).build())
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
            "alternatives",
            "Alternatives",
            "Request alternative routes",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_success",
            "Success",
            "Triggered when route planning completes successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when route planning fails",
            VariableType::Execution,
        );
        node.add_output_pin(
            "route",
            "Route",
            "The primary calculated route",
            VariableType::Struct,
        )
        .set_schema::<RouteResult>();

        node.add_output_pin(
            "alternatives_out",
            "Alternative Routes",
            "Alternative routes if requested",
            VariableType::Struct,
        )
        .set_schema::<RouteResult>();

        node.add_output_pin(
            "distance",
            "Distance",
            "Total route distance in meters",
            VariableType::Float,
        );

        node.add_output_pin(
            "duration",
            "Duration",
            "Estimated travel time in seconds",
            VariableType::Float,
        );

        geometry_output(
            &mut node,
            "geometry_out",
            "Route Geometry",
            "Primary route as a LineString geometry. Unset when no route is found.",
            Some(GeometryKind::LineString),
        );
        geometry_output(
            &mut node,
            "route_geometries",
            "Route Geometries",
            "LineString geometries for all returned routes, with the primary route first.",
            Some(GeometryKind::LineString),
        )
        .set_value_type(ValueType::Array);

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
        crate::geo::pins::clear_output(context, "geometry_out").await?;
        crate::geo::pins::clear_output(context, "route_geometries").await?;

        let start = crate::geo::pins::coordinate_input(context, "start_geometry").await?;
        let end = crate::geo::pins::coordinate_input(context, "end_geometry").await?;
        let waypoints = crate::geo::pins::coordinates_input(context, "waypoint_geometries").await?;
        let profile: String = context.evaluate_pin("profile").await?;
        let alternatives: bool = context.evaluate_pin("alternatives").await?;

        let profile_str = match profile.as_str() {
            "Car" | "car" => "driving",
            "Bike" | "bike" => "cycling",
            "Foot" | "foot" => "foot",
            _ => return Err(flow_like_types::anyhow!("Unsupported profile: {}", profile)),
        };

        let mut coordinates = vec![start];
        coordinates.extend(waypoints);
        coordinates.push(end);
        let coords_str = build_coordinate_string(&coordinates);

        let client = reqwest::Client::builder()
            .user_agent("FlowLike/1.0")
            .build()?;

        let url = format!(
            "https://router.project-osrm.org/route/v1/{}/{}?overview=full&geometries=geojson&steps=true&alternatives={}",
            profile_str, coords_str, alternatives
        );

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(flow_like_types::anyhow!(
                "OSRM API returned status: {}",
                response.status()
            ));
        }

        let body: OsrmResponse = response.json().await?;

        if body.code != "Ok" {
            return Err(flow_like_types::anyhow!(
                "OSRM returned error: {}",
                body.message.unwrap_or_else(|| body.code.clone())
            ));
        }

        let routes = map_osrm_routes(body.routes.unwrap_or_default());
        crate::geo::routing::osrm::set_route_geometries(context, &routes).await?;

        let primary_route = routes.first().cloned().unwrap_or_default();
        let alt_routes: Vec<RouteResult> = routes.into_iter().skip(1).collect();

        context.set_pin_value("route", json!(primary_route)).await?;
        context
            .set_pin_value("alternatives_out", json!(alt_routes))
            .await?;
        context
            .set_pin_value("distance", json!(primary_route.distance))
            .await?;
        context
            .set_pin_value("duration", json!(primary_route.duration))
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
struct OsrmResponse {
    code: String,
    message: Option<String>,
    routes: Option<Vec<OsrmRoute>>,
}
