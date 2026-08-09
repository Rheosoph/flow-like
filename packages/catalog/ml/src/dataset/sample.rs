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
use flow_like_storage::lancedb::query::ExecutableQuery;
#[cfg(feature = "execute")]
use flow_like_types::rand::{self, seq::SliceRandom};
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;

#[crate::register_node]
#[derive(Default)]
pub struct SampleDatasetNode {}

impl SampleDatasetNode {
    pub fn new() -> Self {
        SampleDatasetNode {}
    }
}

#[async_trait]
impl NodeLogic for SampleDatasetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_dataset_sample",
            "Sample Dataset",
            "Random sample N records or a ratio from a dataset",
            "AI/ML/Dataset",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts the sampling",
            VariableType::Execution,
        );

        node.add_input_pin(
            "sample_count",
            "Sample Count",
            "Number of records to sample (if set, takes precedence over ratio)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "sample_ratio",
            "Sample Ratio",
            "Ratio of records to sample (0.0 to 1.0, used if sample_count is 0)",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.1)));

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
            "Destination database connection that receives the sampled rows",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the sampling has finished",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sampled_count",
            "Sampled Count",
            "Number of records that were sampled",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: NodeDBConnection = context.evaluate_pin("source").await?;
        let target: NodeDBConnection = context.evaluate_pin("target").await?;
        let sample_count: i64 = context.evaluate_pin("sample_count").await?;
        let sample_ratio: f64 = context.evaluate_pin("sample_ratio").await?;

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

        let total_count = all_items.len();

        // Determine sample size
        let sample_size = if sample_count > 0 {
            (sample_count as usize).min(total_count)
        } else {
            ((total_count as f64) * sample_ratio).round() as usize
        };

        // Shuffle and take sample
        let sampled_items = {
            let mut rng = rand::rng();
            all_items.shuffle(&mut rng);
            all_items.into_iter().take(sample_size).collect::<Vec<_>>()
        };

        let sampled_count = sampled_items.len();

        // Insert sampled items
        if !sampled_items.is_empty() {
            target.insert_from(context, sampled_items).await?;
        }

        context
            .set_pin_value("sampled_count", json!(sampled_count as i64))
            .await?;
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
