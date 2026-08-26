use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::{FlowCache, cache_delete};

#[crate::register_node]
#[derive(Default)]
pub struct DeleteCacheNode {}

impl DeleteCacheNode {
    pub fn new() -> Self {
        DeleteCacheNode {}
    }
}

#[async_trait]
impl NodeLogic for DeleteCacheNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cache_delete",
            "Delete Cache Entry",
            "Removes a value from the app's cache.",
            "Data/Cache",
        );
        node.set_flowscript_name("data.cache", "delete");
        node.set_receiver("cache");
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

        node.add_input_pin("key", "Key", "The key to remove", VariableType::String);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "deleted",
            "Deleted",
            "True when an entry was actually removed",
            VariableType::Boolean,
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

        let deleted = cache_delete(context, &cache, &key).await?;

        context.set_pin_value("deleted", json!(deleted)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
