use super::config::MemoryConfig;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct OptimizeMemoryNode {}

impl OptimizeMemoryNode {
    pub fn new() -> Self {
        OptimizeMemoryNode {}
    }
}

#[async_trait]
impl NodeLogic for OptimizeMemoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "memory_optimize",
            "Optimize Memory",
            "Runs LanceDB maintenance on the memory table: flush buffered writes, compact fragments, prune old versions, and rebuild indices. Run periodically or after bulk writes.",
            "AI/Memory",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_long_running(true);

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(4)
                .set_governance(10)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "memory_config",
            "Memory Config",
            "MemoryConfig from Create Memory Config node",
            VariableType::Struct,
        )
        .set_schema::<MemoryConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "keep_versions",
            "Keep Versions",
            "Whether to keep old row versions (false = prune for disk savings)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when optimization completes",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let config: MemoryConfig = context.evaluate_pin("memory_config").await?;
        let keep_versions: bool = context.evaluate_pin("keep_versions").await.unwrap_or(false);

        let cached_db = config.database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let db = cached_db.db.read().await;
        db.optimize(keep_versions).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
