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
use flow_like_types::rand::{self, Rng};
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
#[cfg(feature = "execute")]
use std::collections::HashMap;

#[crate::register_node]
#[derive(Default)]
pub struct StratifiedSplitNode {}

impl StratifiedSplitNode {
    pub fn new() -> Self {
        StratifiedSplitNode {}
    }
}

#[async_trait]
impl NodeLogic for StratifiedSplitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_dataset_stratified_split",
            "Stratified Split",
            "Split a dataset into training and testing subsets while maintaining class distribution",
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
            "Execution trigger that starts the stratified split",
            VariableType::Execution,
        );

        node.add_input_pin(
            "split",
            "Split Ratio",
            "Ratio used for assigning rows to the training set (rest goes to test)",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "label_column",
            "Label Column",
            "Name of the column containing class labels for stratification",
            VariableType::String,
        )
        .set_default_value(Some(json!("label")));

        node.add_input_pin(
            "source",
            "Data Source",
            "Data Source (DB or CSV)",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "train",
            "Training Database",
            "Destination database connection that receives the training rows",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "test",
            "Test Database",
            "Destination database connection that receives the testing rows",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the stratified split has finished",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: NodeDBConnection = context.evaluate_pin("source").await?;
        let test: NodeDBConnection = context.evaluate_pin("test").await?;
        let train: NodeDBConnection = context.evaluate_pin("train").await?;
        let probability: f64 = context.evaluate_pin("split").await?;
        let label_column: String = context.evaluate_pin("label_column").await?;

        let source = source.load(context).await?;
        let test = test.load(context).await?;
        let train = train.load(context).await?;

        source.ensure_flushed().await?;
        let source_guard = source.db.read().await;
        let source_table = source_guard.inner().raw().await?;
        let query = source_table.query();
        let mut item_stream = query.execute().await?;

        // Group items by class label
        let mut class_buckets: HashMap<String, Vec<flow_like_types::Value>> = HashMap::new();

        while let Ok(Some(items)) = item_stream.try_next().await {
            let items = record_batch_to_value(&items)?;

            for item in items {
                let label = item
                    .get(&label_column)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        item.get(&label_column)
                            .map(|v| v.to_string())
                            .unwrap_or_default()
                    });

                class_buckets.entry(label).or_default().push(item);
            }
        }

        // Split each class proportionally - collect all items first to avoid holding RNG across await
        let (all_train_items, all_test_items) = {
            let mut rng = rand::rng();
            let mut train_items = Vec::new();
            let mut test_items = Vec::new();

            for (_label, items) in class_buckets {
                for item in items {
                    if rng.random_bool(probability) {
                        train_items.push(item);
                    } else {
                        test_items.push(item);
                    }
                }
            }
            (train_items, test_items)
        };

        if !all_train_items.is_empty() {
            train.db.write().await.insert(all_train_items).await?;
        }
        if !all_test_items.is_empty() {
            test.db.write().await.insert(all_test_items).await?;
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
