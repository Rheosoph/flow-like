use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::{FlowCache, cache_get_or_set};

#[crate::register_node]
#[derive(Default)]
pub struct GetOrWriteCacheNode {}

impl GetOrWriteCacheNode {
    pub fn new() -> Self {
        GetOrWriteCacheNode {}
    }
}

#[async_trait]
impl NodeLogic for GetOrWriteCacheNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cache_get_or_write",
            "Get or Write Cache",
            "Returns the cached value, or stores the fallback and returns that. Exactly one caller gets Written = true, even when several runs reach this node at the same moment.",
            "Data/Cache",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

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

        node.add_input_pin("key", "Key", "The key to read or claim", VariableType::String);

        node.add_input_pin(
            "fallback",
            "Fallback",
            "Value to store when the key holds nothing live",
            VariableType::Struct,
        );

        node.add_input_pin(
            "ttl_seconds",
            "Lifetime (s)",
            "Seconds until a newly written entry expires. 0 keeps it until it is deleted.",
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
            "value",
            "Value",
            "The value now held under the key",
            VariableType::Struct,
        );

        node.add_output_pin(
            "written",
            "Written",
            "True when this run is the one that stored the fallback. Branch on this to do expensive work only once.",
            VariableType::Boolean,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let cache: FlowCache = context.evaluate_pin("cache").await?;
        let key: String = context.evaluate_pin("key").await?;
        let fallback: Value = context.evaluate_pin("fallback").await?;
        let ttl_seconds: i64 = context.evaluate_pin("ttl_seconds").await?;

        if ttl_seconds < 0 {
            return Err(flow_like_types::anyhow!(
                "Cache lifetime must not be negative; use 0 to keep the entry indefinitely"
            ));
        }

        let (value, written) =
            cache_get_or_set(context, &cache, &key, fallback, Some(ttl_seconds as u64)).await?;

        context.set_pin_value("value", value).await?;
        context.set_pin_value("written", json!(written)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
