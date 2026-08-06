use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, PinType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::{CacheScope, FlowCache};

const SCOPE_APP: &str = "App";
const SCOPE_USER: &str = "User";

#[crate::register_node]
#[derive(Default)]
pub struct OpenCacheNode {}

impl OpenCacheNode {
    pub fn new() -> Self {
        OpenCacheNode {}
    }
}

#[async_trait]
impl NodeLogic for OpenCacheNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cache_open",
            "Open Cache",
            "Opens the app's key/value cache. Connect the result to Read, Write and Delete Cache nodes.",
            "Data/Cache",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

        node.add_input_pin(
            "scope",
            "Scope",
            "App shares entries with everyone who can run this app. User keeps them private to whoever triggered the run.",
            VariableType::String,
        )
        .set_default_value(Some(json!(SCOPE_APP)))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![SCOPE_APP.to_string(), SCOPE_USER.to_string()])
                .build(),
        );

        node.add_input_pin(
            "namespace",
            "Namespace",
            "Optional prefix so short keys from different flows cannot collide",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "cache",
            "Cache",
            "Cache handle for the Read, Write and Delete Cache nodes",
            VariableType::Struct,
        )
        .set_schema::<FlowCache>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let scope: String = context.evaluate_pin("scope").await?;
        let namespace: String = context.evaluate_pin("namespace").await?;

        let cache = FlowCache {
            scope: CacheScope::from_label(&scope),
            namespace: namespace.trim().to_string(),
        };

        context.set_pin_value("cache", json!(cache)).await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &flow_like::flow::board::Board) {
        // Surface the chosen scope on the node title so a board reviewer can see whether
        // a flow is reading shared or per-user state without opening the node.
        let scope = node
            .pins
            .iter()
            .find(|(_, pin)| pin.name == "scope" && pin.pin_type == PinType::Input)
            .and_then(|(_, pin)| pin.default_value.as_ref())
            .and_then(|raw| flow_like_types::json::from_slice::<String>(raw).ok())
            .unwrap_or_else(|| SCOPE_APP.to_string());

        node.friendly_name = match CacheScope::from_label(&scope) {
            CacheScope::User => "Open Cache (User)".to_string(),
            CacheScope::App => "Open Cache (App)".to_string(),
        };
    }
}
