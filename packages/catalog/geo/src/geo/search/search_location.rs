use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, geometry::GeometryKind, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::{BoundingBox, GeoCoordinate, pins};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct SearchResult {
    pub display_name: String,
    pub coordinate: GeoCoordinate,
    pub place_type: String,
    pub importance: f64,
    pub bounding_box: Option<BoundingBox>,
    pub osm_id: Option<i64>,
    pub osm_type: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct SearchLocationNode {}

impl SearchLocationNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchLocationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_search_location",
            "Search Location",
            "Searches for a location by name or address using the Nominatim geocoding service (OpenStreetMap). Returns matching locations with coordinates.",
            "Web/Geo/Search",
        );
        node.set_version(1);
        node.set_flowscript_name("geo", "searchLocation");
        node.add_icon("/flow/icons/map.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the location search",
            VariableType::Execution,
        );
        node.add_input_pin(
            "query",
            "Query",
            "The search query (address, place name, etc.)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of results to return. Default: 5",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "country_codes",
            "Country Codes",
            "Optional comma-separated list of country codes to limit search (e.g., 'de,at,ch')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_success",
            "Success",
            "Triggered when the search completes successfully",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when the search fails",
            VariableType::Execution,
        );
        node.add_output_pin(
            "results",
            "Results",
            "Array of search results with coordinates",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<SearchResult>();

        node.add_output_pin(
            "first_result",
            "First Result",
            "The first/best matching result (if any)",
            VariableType::Struct,
        )
        .set_schema::<SearchResult>();

        pins::geometry_output(
            &mut node,
            "geometry_out",
            "First Geometry",
            "Point for the first match. Unset when no location matches.",
            Some(GeometryKind::Point),
        );
        pins::geometry_output(
            &mut node,
            "geometries",
            "Geometries",
            "Points for all matches, in the same order as Results",
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
        for name in ["geometry_out", "geometries"] {
            pins::clear_output(context, name).await?;
        }

        let query: String = context.evaluate_pin("query").await?;
        let limit: i64 = context.evaluate_pin("limit").await?;
        let country_codes: String = context.evaluate_pin("country_codes").await?;

        if query.trim().is_empty() {
            return Err(flow_like_types::anyhow!("Search query cannot be empty"));
        }

        let limit = limit.clamp(1, 50) as u32;

        let client = reqwest::Client::builder()
            .user_agent("FlowLike/1.0")
            .build()?;

        let mut url = format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=json&limit={}&addressdetails=1",
            urlencoding::encode(&query),
            limit
        );

        if !country_codes.trim().is_empty() {
            url.push_str(&format!("&countrycodes={}", country_codes.trim()));
        }

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(flow_like_types::anyhow!(
                "Nominatim API returned status: {}",
                response.status()
            ));
        }

        let body: Vec<NominatimResult> = response.json().await?;

        publish_results(context, body).await?;

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
async fn publish_results(
    context: &mut ExecutionContext,
    body: Vec<NominatimResult>,
) -> flow_like_types::Result<()> {
    let results: Vec<SearchResult> = body
        .into_iter()
        .map(|r| {
            Ok(SearchResult {
                display_name: r.display_name,
                coordinate: GeoCoordinate::new(r.lat.parse()?, r.lon.parse()?),
                place_type: r.r#type,
                importance: r.importance,
                bounding_box: r.boundingbox.map(|bb| {
                    if bb.len() == 4 {
                        BoundingBox::new(
                            bb[0].parse().unwrap_or(0.0),
                            bb[2].parse().unwrap_or(0.0),
                            bb[1].parse().unwrap_or(0.0),
                            bb[3].parse().unwrap_or(0.0),
                        )
                    } else {
                        BoundingBox::default()
                    }
                }),
                osm_id: r.osm_id,
                osm_type: r.osm_type,
            })
        })
        .collect::<flow_like_types::Result<_>>()?;

    let geometries = results
        .iter()
        .map(|result| pins::point_geometry(&result.coordinate))
        .collect::<flow_like_types::Result<Vec<_>>>()?;

    let first_result = results.first().cloned().unwrap_or_default();

    context.set_pin_value("results", json!(results)).await?;
    context
        .set_pin_value("first_result", json!(first_result))
        .await?;
    if let Some(first) = geometries.first() {
        context.set_pin_value("geometry_out", first.clone()).await?;
    } else {
        pins::clear_output(context, "geometry_out").await?;
    }
    context
        .set_pin_value("geometries", json!(geometries))
        .await?;

    Ok(())
}

#[cfg(feature = "execute")]
#[derive(Deserialize)]
struct NominatimResult {
    display_name: String,
    lat: String,
    lon: String,
    #[serde(rename = "type")]
    r#type: String,
    importance: f64,
    boundingbox: Option<Vec<String>>,
    osm_id: Option<i64>,
    osm_type: Option<String>,
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_types::Value;
    use std::sync::Arc;

    #[tokio::test]
    async fn search_geometries_match_results_and_empty_search_clears_first_point() {
        let mut context = pins::tests::execution_context(Arc::new(SearchLocationNode::new())).await;
        let body = flow_like_types::json::from_value(json!([
            {"display_name":"Berlin", "lat":"52.52", "lon":"13.405", "type":"city", "importance":0.9},
            {"display_name":"New York", "lat":"40.7", "lon":"-74", "type":"city", "importance":0.8}
        ])).unwrap();
        publish_results(&mut context, body).await.unwrap();
        let geometries: Vec<Value> = context.evaluate_pin("geometries").await.unwrap();
        assert_eq!(
            geometries,
            vec![
                json!({"type":"Point","coordinates":[13.405,52.52]}),
                json!({"type":"Point","coordinates":[-74.0,40.7]}),
            ]
        );
        assert_eq!(
            context.evaluate_pin::<Value>("geometry_out").await.unwrap(),
            geometries[0]
        );
        let results: Vec<SearchResult> = context.evaluate_pin("results").await.unwrap();
        assert_eq!(results[1].coordinate.longitude, -74.0);

        let first = context.get_pin_by_name("geometry_out").await.unwrap();
        context.override_pin_value(first.id(), geometries[0].clone());
        publish_results(&mut context, Vec::new()).await.unwrap();
        assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
        assert_eq!(
            context.evaluate_pin::<Value>("geometries").await.unwrap(),
            json!([])
        );
        assert_eq!(
            context.evaluate_pin::<Value>("results").await.unwrap(),
            json!([])
        );
    }

    #[tokio::test]
    async fn search_rejects_malformed_coordinates_instead_of_emitting_a_zero_point() {
        for latitude in ["bad", "NaN", "91"] {
            let mut context =
                pins::tests::execution_context(Arc::new(SearchLocationNode::new())).await;
            let body = flow_like_types::json::from_value(json!([
                {"display_name":"Invalid", "lat":latitude, "lon":"13.405", "type":"city", "importance":0.9}
            ])).unwrap();
            assert!(publish_results(&mut context, body).await.is_err());
            assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
            assert!(context.evaluate_pin::<Value>("geometries").await.is_err());
        }
    }
}
