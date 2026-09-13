use crate::geo::GeoCoordinate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub enum RouteProfile {
    #[default]
    Car,
    Bike,
    Foot,
}

impl RouteProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            RouteProfile::Car => "driving",
            RouteProfile::Bike => "cycling",
            RouteProfile::Foot => "foot",
        }
    }
}

pub fn build_coordinate_string(coordinates: &[GeoCoordinate]) -> String {
    coordinates
        .iter()
        .map(|coord| format!("{},{}", coord.longitude, coord.latitude))
        .collect::<Vec<String>>()
        .join(";")
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct RouteGeometry {
    pub points: Vec<GeoCoordinate>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct RouteStep {
    pub instruction: String,
    pub distance: f64,
    pub duration: f64,
    pub name: String,
    pub maneuver_type: String,
    pub coordinate: GeoCoordinate,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct RouteLeg {
    pub distance: f64,
    pub duration: f64,
    pub summary: String,
    pub steps: Vec<RouteStep>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct RouteResult {
    pub distance: f64,
    pub duration: f64,
    pub geometry: RouteGeometry,
    pub legs: Vec<RouteLeg>,
    pub weight_name: String,
}

#[derive(Deserialize)]
pub struct OsrmRoute {
    pub distance: f64,
    pub duration: f64,
    pub geometry: OsrmGeometry,
    pub legs: Vec<OsrmLeg>,
    pub weight_name: String,
}

#[derive(Deserialize)]
pub struct OsrmGeometry {
    pub coordinates: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
pub struct OsrmLeg {
    pub distance: f64,
    pub duration: f64,
    pub summary: String,
    pub steps: Vec<OsrmStep>,
}

#[derive(Deserialize)]
pub struct OsrmStep {
    pub distance: f64,
    pub duration: f64,
    pub name: String,
    pub maneuver: Option<OsrmManeuver>,
}

#[derive(Deserialize)]
pub struct OsrmManeuver {
    #[serde(rename = "type")]
    pub r#type: String,
    pub modifier: Option<String>,
    pub location: [f64; 2],
}

pub fn map_osrm_routes(routes: Vec<OsrmRoute>) -> Vec<RouteResult> {
    routes
        .into_iter()
        .map(|route| {
            let geometry = route
                .geometry
                .coordinates
                .iter()
                .map(|c| GeoCoordinate::new(c[1], c[0]))
                .collect::<Vec<GeoCoordinate>>();

            let legs = route
                .legs
                .into_iter()
                .map(|leg| {
                    let steps = leg
                        .steps
                        .into_iter()
                        .map(|step| RouteStep {
                            instruction: step
                                .maneuver
                                .as_ref()
                                .map(|m| m.modifier.clone().unwrap_or_else(|| m.r#type.clone()))
                                .unwrap_or_default(),
                            distance: step.distance,
                            duration: step.duration,
                            name: step.name,
                            maneuver_type: step
                                .maneuver
                                .as_ref()
                                .map(|m| m.r#type.clone())
                                .unwrap_or_default(),
                            coordinate: step
                                .maneuver
                                .as_ref()
                                .map(|m| GeoCoordinate::new(m.location[1], m.location[0]))
                                .unwrap_or_default(),
                        })
                        .collect();

                    RouteLeg {
                        distance: leg.distance,
                        duration: leg.duration,
                        summary: leg.summary,
                        steps,
                    }
                })
                .collect();

            RouteResult {
                distance: route.distance,
                duration: route.duration,
                geometry: RouteGeometry { points: geometry },
                legs,
                weight_name: route.weight_name,
            }
        })
        .collect()
}

#[cfg(feature = "execute")]
pub(crate) async fn set_route_geometries(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    routes: &[RouteResult],
) -> flow_like_types::Result<()> {
    use flow_like_types::json::json;

    let geometries = routes
        .iter()
        .map(|route| crate::geo::pins::line_geometry(&route.geometry.points))
        .collect::<flow_like_types::Result<Vec<_>>>()?;
    if let Some(primary) = geometries.first() {
        context
            .set_pin_value("geometry_out", primary.clone())
            .await?;
    } else {
        crate::geo::pins::clear_output(context, "geometry_out").await?;
    }
    context
        .set_pin_value("route_geometries", json!(geometries))
        .await?;
    Ok(())
}
