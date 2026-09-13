use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, geometry::GeometryKind, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::{GeoCoordinate, pins};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct ReverseGeocodeResult {
    pub display_name: String,
    pub coordinate: GeoCoordinate,
    pub address: Address,
    pub osm_id: Option<i64>,
    pub osm_type: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct Address {
    pub house_number: Option<String>,
    pub road: Option<String>,
    pub suburb: Option<String>,
    pub city: Option<String>,
    pub county: Option<String>,
    pub state: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct ReverseGeocodeNode {}

impl ReverseGeocodeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReverseGeocodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_reverse_geocode",
            "Reverse Geocode",
            "Converts geographic coordinates to a human-readable address using the Nominatim service (OpenStreetMap).",
            "Web/Geo/Search",
        );
        node.set_version(2);
        node.set_flowscript_name("geo", "reverseGeocode");
        node.add_icon("/flow/icons/map.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate reverse geocoding",
            VariableType::Execution,
        );
        pins::geometry_input(
            &mut node,
            "geometry",
            "Geometry",
            "Point to look up",
            Some(GeometryKind::Point),
        );

        node.add_input_pin(
            "zoom",
            "Detail Level",
            "Level of detail for the address (0-18). Higher = more specific. Default: 18",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(18)));

        node.add_output_pin(
            "exec_success",
            "Success",
            "Triggered when reverse geocoding completes successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when reverse geocoding fails",
            VariableType::Execution,
        );
        node.add_output_pin(
            "result",
            "Result",
            "The reverse geocoding result with address details",
            VariableType::Struct,
        )
        .set_schema::<ReverseGeocodeResult>();

        node.add_output_pin(
            "display_name",
            "Display Name",
            "The full formatted address string",
            VariableType::String,
        );

        pins::geometry_output(
            &mut node,
            "geometry_out",
            "Result Geometry",
            "Point returned by the geocoding service",
            Some(GeometryKind::Point),
        );

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
        pins::clear_output(context, "geometry_out").await?;

        let coordinate = pins::coordinate_input(context, "geometry").await?;
        let zoom: i64 = context.evaluate_pin("zoom").await?;
        let zoom = zoom.clamp(0, 18);

        let client = reqwest::Client::builder()
            .user_agent("FlowLike/1.0")
            .build()?;

        let url = reverse_url(&coordinate, zoom);

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(flow_like_types::anyhow!(
                "Nominatim API returned status: {}",
                response.status()
            ));
        }

        let body: NominatimReverseResult = response.json().await?;

        publish_result(context, body).await?;

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
fn reverse_url(coordinate: &GeoCoordinate, zoom: i64) -> String {
    format!(
        "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json&addressdetails=1&zoom={}",
        coordinate.latitude, coordinate.longitude, zoom
    )
}

#[cfg(feature = "execute")]
async fn publish_result(
    context: &mut ExecutionContext,
    body: NominatimReverseResult,
) -> flow_like_types::Result<()> {
    let address = body
        .address
        .map(|a| Address {
            house_number: a.house_number,
            road: a.road,
            suburb: a.suburb,
            city: a.city.or(a.town).or(a.village),
            county: a.county,
            state: a.state,
            postcode: a.postcode,
            country: a.country,
            country_code: a.country_code,
        })
        .unwrap_or_default();

    let result = ReverseGeocodeResult {
        display_name: body.display_name.clone(),
        coordinate: GeoCoordinate::new(body.lat.parse()?, body.lon.parse()?),
        address,
        osm_id: body.osm_id,
        osm_type: body.osm_type,
    };

    let geometry = pins::point_geometry(&result.coordinate)?;
    context.set_pin_value("result", json!(result)).await?;
    context
        .set_pin_value("display_name", json!(body.display_name))
        .await?;
    context.set_pin_value("geometry_out", geometry).await?;

    Ok(())
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct NominatimReverseResult {
    display_name: String,
    lat: String,
    lon: String,
    address: Option<NominatimAddress>,
    osm_id: Option<i64>,
    osm_type: Option<String>,
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct NominatimAddress {
    house_number: Option<String>,
    road: Option<String>,
    suburb: Option<String>,
    city: Option<String>,
    town: Option<String>,
    village: Option<String>,
    county: Option<String>,
    state: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_types::Value;
    use std::sync::Arc;

    #[tokio::test]
    async fn geometry_drives_reverse_request_and_response_exposes_returned_point() {
        let mut context = pins::tests::execution_context(Arc::new(ReverseGeocodeNode::new())).await;
        context
            .set_pin_value(
                "geometry",
                json!({"type":"Point","coordinates":[13.405,52.52]}),
            )
            .await
            .unwrap();
        let coordinate = pins::coordinate_input(&context, "geometry").await.unwrap();
        assert_eq!(
            reverse_url(&coordinate, 18),
            "https://nominatim.openstreetmap.org/reverse?lat=52.52&lon=13.405&format=json&addressdetails=1&zoom=18"
        );

        let body = flow_like_types::json::from_value(json!({
            "display_name":"Berlin, Germany", "lat":"52.5201", "lon":"13.4051",
            "address":{"city":"Berlin","country":"Germany"}
        }))
        .unwrap();
        publish_result(&mut context, body).await.unwrap();
        let geometry = context.evaluate_pin::<Value>("geometry_out").await.unwrap();
        assert_eq!(
            geometry,
            json!({"type":"Point","coordinates":[13.4051,52.5201]})
        );
        let result: ReverseGeocodeResult = context.evaluate_pin("result").await.unwrap();
        assert_eq!(result.address.city.as_deref(), Some("Berlin"));
        assert_eq!(result.coordinate.longitude, geometry["coordinates"][0]);

        // A later input failure must clear the previous returned point before requesting a location.
        context
            .set_pin_value("zoom", json!("invalid"))
            .await
            .unwrap();
        assert!(ReverseGeocodeNode::new().run(&mut context).await.is_err());
        assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
    }

    #[tokio::test]
    async fn reverse_geocode_rejects_invalid_response_coordinates() {
        for longitude in ["bad", "NaN", "181"] {
            let mut context =
                pins::tests::execution_context(Arc::new(ReverseGeocodeNode::new())).await;
            let body = flow_like_types::json::from_value(json!({
                "display_name":"Invalid", "lat":"52.52", "lon":longitude
            }))
            .unwrap();
            assert!(publish_result(&mut context, body).await.is_err());
            assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
        }
    }
}
