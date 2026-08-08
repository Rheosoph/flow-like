use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::{FlowCache, cache_invalidate_namespace};

#[crate::register_node]
#[derive(Default)]
pub struct InvalidateNamespaceNode {}

impl InvalidateNamespaceNode {
    pub fn new() -> Self {
        InvalidateNamespaceNode {}
    }
}

#[async_trait]
impl NodeLogic for InvalidateNamespaceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cache_invalidate_namespace",
            "Invalidate Cache Namespace",
            "Removes every entry in the cache handle's namespace in one call — including entries with no lifetime. The handle must carry a namespace; per-key removal is the Delete Cache node's job.",
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
            "Cache handle from the Open Cache node. Its namespace decides what is removed.",
            VariableType::Struct,
        )
        .set_schema::<FlowCache>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "deleted",
            "Deleted",
            "How many entries were removed",
            VariableType::Integer,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let cache: FlowCache = context.evaluate_pin("cache").await?;
        let deleted = cache_invalidate_namespace(context, &cache).await?;

        context.set_pin_value("deleted", json!(deleted)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
