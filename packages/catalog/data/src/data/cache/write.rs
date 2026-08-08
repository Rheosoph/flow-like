use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::{FlowCache, cache_set};

#[crate::register_node]
#[derive(Default)]
pub struct WriteCacheNode {}

impl WriteCacheNode {
    pub fn new() -> Self {
        WriteCacheNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteCacheNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cache_write",
            "Write Cache",
            "Stores a value in the app's cache, optionally with a lifetime after which it disappears on its own. The cache is for small, hot values (about 1 MB max) — persist large data to the app's storage instead.",
            "Data/Cache",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(2);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "cache",
            "Cache",
            "Cache handle from the Open Cache node",
            VariableType::Struct,
        )
        .set_schema::<FlowCache>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("key", "Key", "The key to write", VariableType::String);

        node.add_input_pin(
            "value",
            "Value",
            "The value to store — a struct, array, string, number or boolean",
            VariableType::Generic,
        );

        node.add_input_pin(
            "ttl_seconds",
            "Lifetime (s)",
            "Seconds until the entry expires. 0 keeps it until it is deleted.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "expires_at",
            "Expires At",
            "Unix timestamp in milliseconds when the entry expires, or 0 when it never does",
            VariableType::Integer,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let cache: FlowCache = context.evaluate_pin("cache").await?;
        let key: String = context.evaluate_pin("key").await?;
        let value: Value = context.evaluate_pin("value").await?;
        let ttl_seconds: i64 = context.evaluate_pin("ttl_seconds").await?;

        if ttl_seconds < 0 {
            return Err(flow_like_types::anyhow!(
                "Cache lifetime must not be negative; use 0 to keep the entry indefinitely"
            ));
        }

        let expires_at = cache_set(context, &cache, &key, value, Some(ttl_seconds as u64)).await?;

        context
            .set_pin_value("expires_at", json!(expires_at.unwrap_or(0)))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
