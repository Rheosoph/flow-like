use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::{FlowCache, cache_get};

#[crate::register_node]
#[derive(Default)]
pub struct ReadCacheNode {}

impl ReadCacheNode {
    pub fn new() -> Self {
        ReadCacheNode {}
    }
}

#[async_trait]
impl NodeLogic for ReadCacheNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cache_read",
            "Read Cache",
            "Reads a value from the app's cache. Reports a miss when the key was never written or its lifetime has elapsed.",
            "Data/Cache",
        );
        node.set_flowscript_name("data.cache", "read");
        node.set_receiver("cache");
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

        node.add_input_pin("key", "Key", "The key to read", VariableType::String);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "found",
            "Found",
            "True when a live entry existed for this key",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "value",
            "Value",
            "The cached value — whatever type was stored — or null on a miss",
            VariableType::Generic,
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

        let hit = cache_get(context, &cache, &key).await?;

        let (found, value) = match hit {
            Some(hit) => (true, hit.value),
            None => (false, Value::Null),
        };

        context.set_pin_value("found", json!(found)).await?;
        context.set_pin_value("value", value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
