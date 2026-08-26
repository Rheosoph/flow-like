use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

use crate::geo::routing::osrm::RouteProfile;

#[crate::register_node]
#[derive(Default)]
pub struct OsrmTileNode {}

impl OsrmTileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for OsrmTileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_osrm_tile",
            "OSRM Tile",
            "Fetches vector map tiles (MVT) from an OSRM server.",
            "Web/Geo/Routing",
        );
        node.set_flowscript_name("geo", "osrmTile");
        node.add_icon("/flow/icons/layers.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the tile request",
            VariableType::Execution,
        );

        node.add_input_pin(
            "profile",
            "Profile",
            "Transportation mode: Car, Bike, or Foot",
            VariableType::Struct,
        )
        .set_schema::<RouteProfile>()
        .set_default_value(Some(json!(RouteProfile::Car)));

        node.add_input_pin("z", "Zoom", "Tile zoom level", VariableType::Integer)
            .set_default_value(Some(json!(14)));

        node.add_input_pin("x", "X", "Tile X coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Tile Y coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "path",
            "Path",
            "Destination path for the MVT tile",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
            "Triggered when the tile is fetched",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Triggered when the tile request fails",
            VariableType::Execution,
        );

        node.add_output_pin(
            "tile_path",
            "Tile Path",
            "Stored tile path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_output_pin(
            "content_type",
            "Content Type",
            "Content type returned by the server",
            VariableType::String,
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

        let profile: RouteProfile = context.evaluate_pin("profile").await?;
        let z: i64 = context.evaluate_pin("z").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;
        let base_url: String = context.evaluate_pin("base_url").await?;

        let profile_str = profile.as_str();
        let base_url = base_url.trim_end_matches('/');

        let url = format!("{}/tile/v1/{}/{}/{}/{}.mvt", base_url, profile_str, z, x, y);

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

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = response.bytes().await?;
        let path = path.set_extension(context, "mvt").await?;
        path.put(context, bytes.to_vec(), false).await?;

        context.set_pin_value("tile_path", json!(path)).await?;
        context
            .set_pin_value("content_type", json!(content_type))
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
