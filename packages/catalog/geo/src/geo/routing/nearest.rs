use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, geometry::GeometryKind, json::json};

use crate::geo::pins::{geometry_input, geometry_output};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::{GeoCoordinate, routing::osrm::RouteProfile};

#[cfg(feature = "execute")]
use crate::geo::routing::osrm::build_coordinate_string;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct NearestWaypoint {
    pub name: String,
    pub distance: f64,
    pub coordinate: GeoCoordinate,
    pub hint: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct OsrmNearestNode {}

impl OsrmNearestNode {
    pub fn new() -> Self {
        Self {}
    }

    #[cfg(feature = "execute")]
    async fn set_waypoints(
        context: &mut ExecutionContext,
        response_waypoints: Vec<OsrmWaypoint>,
    ) -> flow_like_types::Result<()> {
        let waypoints: Vec<NearestWaypoint> = response_waypoints
            .into_iter()
            .map(|wp| NearestWaypoint {
                name: wp.name,
                distance: wp.distance.unwrap_or_default(),
                coordinate: GeoCoordinate::new(wp.location[1], wp.location[0]),
                hint: wp.hint,
            })
            .collect();

        let nearest = waypoints.first().cloned().unwrap_or_default();

        let waypoint_geometries = waypoints
            .iter()
            .map(|waypoint| crate::geo::pins::point_geometry(&waypoint.coordinate))
            .collect::<flow_like_types::Result<Vec<_>>>()?;
        if let Some(geometry) = waypoint_geometries.first() {
            context
                .set_pin_value("geometry_out", geometry.clone())
                .await?;
        } else {
            crate::geo::pins::clear_output(context, "geometry_out").await?;
        }
        context
            .set_pin_value("waypoint_geometries", json!(waypoint_geometries))
            .await?;

        context.set_pin_value("nearest", json!(nearest)).await?;
        context.set_pin_value("waypoints", json!(waypoints)).await?;

        Ok(())
    }
}

#[async_trait]
impl NodeLogic for OsrmNearestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_osrm_nearest",
            "OSRM Nearest",
            "Finds the nearest routable point(s) to a coordinate using OSRM.",
            "Web/Geo/Routing",
        );
        node.set_version(2);
        node.set_flowscript_name("geo", "osrmNearest");
        node.add_icon("/flow/icons/map-pin.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the nearest-point lookup",
            VariableType::Execution,
        );

        geometry_input(
            &mut node,
            "geometry",
            "Geometry",
            "Point geometry to snap to the road network",
            Some(GeometryKind::Point),
        );

        node.add_input_pin(
            "profile",
            "Profile",
            "Transportation mode: Car, Bike, or Foot",
            VariableType::Struct,
        )
        .set_schema::<RouteProfile>()
        .set_default_value(Some(json!(RouteProfile::Car)));

        node.add_input_pin(
            "number",
            "Number",
            "Maximum number of nearest points to return (1-50)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

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
            "Triggered when the nearest-point lookup succeeds",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when the nearest-point lookup fails",
            VariableType::Execution,
        );

        node.add_output_pin(
            "nearest",
            "Nearest",
            "The closest routable point",
            VariableType::Struct,
        )
        .set_schema::<NearestWaypoint>();

        node.add_output_pin(
            "waypoints",
            "Waypoints",
            "List of nearest routable points",
            VariableType::Struct,
        )
        .set_schema::<NearestWaypoint>()
        .set_value_type(ValueType::Array);

        geometry_output(
            &mut node,
            "geometry_out",
            "Nearest Geometry",
            "Closest routable Point geometry. Unset when no point is found.",
            Some(GeometryKind::Point),
        );
        geometry_output(
            &mut node,
            "waypoint_geometries",
            "Waypoint Geometries",
            "Nearest routable Point geometries in the same order as Waypoints.",
            Some(GeometryKind::Point),
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
        crate::geo::pins::clear_output(context, "waypoint_geometries").await?;

        let coordinate = crate::geo::pins::coordinate_input(context, "geometry").await?;
        let profile: RouteProfile = context.evaluate_pin("profile").await?;
        let number: i64 = context.evaluate_pin("number").await?;
        let base_url: String = context.evaluate_pin("base_url").await?;

        let number = number.clamp(1, 50);
        let coords = build_coordinate_string(&[coordinate]);
        let profile_str = profile.as_str();
        let base_url = base_url.trim_end_matches('/');

        let url = format!(
            "{}/nearest/v1/{}/{}?number={}",
            base_url, profile_str, coords, number
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

        let body: OsrmNearestResponse = response.json().await?;
        if body.code != "Ok" {
            return Err(flow_like_types::anyhow!(
                "OSRM returned error: {}",
                body.message.unwrap_or_else(|| body.code.clone())
            ));
        }

        Self::set_waypoints(context, body.waypoints.unwrap_or_default()).await?;

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
struct OsrmNearestResponse {
    code: String,
    message: Option<String>,
    waypoints: Option<Vec<OsrmWaypoint>>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct OsrmWaypoint {
    name: String,
    location: [f64; 2],
    distance: Option<f64>,
    hint: Option<String>,
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::geo::pins::tests::execution_context;
    use flow_like_types::Value;
    use std::sync::Arc;

    #[tokio::test]
    async fn nearest_service_response_exposes_points_and_empty_results_clear_primary() {
        let response: OsrmNearestResponse = flow_like_types::json::from_value(json!({
            "code":"Ok",
            "waypoints":[
                {"name":"First Street","location":[13.405,52.52],"distance":2.0,"hint":"first"},
                {"name":"Second Street","location":[13.41,52.53],"distance":3.0,"hint":"second"}
            ]
        }))
        .unwrap();
        let mut context = execution_context(Arc::new(OsrmNearestNode::new())).await;
        OsrmNearestNode::set_waypoints(&mut context, response.waypoints.unwrap())
            .await
            .unwrap();
        let point: Value = context.evaluate_pin("geometry_out").await.unwrap();
        assert_eq!(point, json!({"type":"Point","coordinates":[13.405,52.52]}));
        let points: Vec<Value> = context.evaluate_pin("waypoint_geometries").await.unwrap();
        assert_eq!(
            points,
            vec![point, json!({"type":"Point","coordinates":[13.41,52.53]})]
        );
        let nearest: NearestWaypoint = context.evaluate_pin("nearest").await.unwrap();
        assert_eq!(nearest.coordinate.latitude, 52.52);
        assert_eq!(nearest.name, "First Street");
        OsrmNearestNode::set_waypoints(&mut context, Vec::new())
            .await
            .unwrap();
        assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
        assert_eq!(
            context
                .evaluate_pin::<Value>("waypoint_geometries")
                .await
                .unwrap(),
            json!([])
        );
    }
}
