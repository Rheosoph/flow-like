use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::arrow_utils::record_batch_to_value;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_storage::lancedb::query::ExecutableQuery;
#[cfg(feature = "execute")]
use flow_like_types::rand::{self, seq::SliceRandom};
use flow_like_types::{Result, async_trait};
#[cfg(feature = "execute")]
use futures::TryStreamExt;

#[crate::register_node]
#[derive(Default)]
pub struct ShuffleDatasetNode {}

impl ShuffleDatasetNode {
    pub fn new() -> Self {
        ShuffleDatasetNode {}
    }
}

#[async_trait]
impl NodeLogic for ShuffleDatasetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_dataset_shuffle",
            "Shuffle Dataset",
            "Shuffle dataset rows randomly",
            "AI/ML/Dataset",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(5)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts the shuffle",
            VariableType::Execution,
        );

        node.add_input_pin(
            "source",
            "Data Source",
            "Data Source (DB or CSV)",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "target",
            "Target Database",
            "Destination database connection that receives the shuffled rows",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the shuffle has finished",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: NodeDBConnection = context.evaluate_pin("source").await?;
        let target: NodeDBConnection = context.evaluate_pin("target").await?;

        let source = source.load(context).await?;
        let target = target.load(context).await?;

        source.ensure_flushed().await?;
        let source_guard = source.db.read().await;
        let source_table = source_guard.inner().raw().await?;
        let query = source_table.query();
        let mut item_stream = query.execute().await?;

        // Collect all items first
        let mut all_items: Vec<flow_like_types::Value> = Vec::new();

        while let Ok(Some(items)) = item_stream.try_next().await {
            let items = record_batch_to_value(&items)?;
            all_items.extend(items);
        }

        // Shuffle the collected items
        {
            let mut rng = rand::rng();
            all_items.shuffle(&mut rng);
        }

        // Insert shuffled items
        if !all_items.is_empty() {
            target.db.write().await.insert(all_items).await?;
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}
