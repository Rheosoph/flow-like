use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geo::{BoundingBox, GeoCoordinate};

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

        let results: Vec<SearchResult> = body
            .into_iter()
            .map(|r| SearchResult {
                display_name: r.display_name,
                coordinate: GeoCoordinate::new(
                    r.lat.parse().unwrap_or(0.0),
                    r.lon.parse().unwrap_or(0.0),
                ),
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
            .collect();

        let first_result = results.first().cloned().unwrap_or_default();

        context.set_pin_value("results", json!(results)).await?;
        context
            .set_pin_value("first_result", json!(first_result))
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
