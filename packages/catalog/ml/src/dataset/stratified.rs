use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::CachedDB;
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::arrow_utils::record_batch_to_value;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_storage::lancedb::query::ExecutableQuery;
#[cfg(feature = "execute")]
use flow_like_types::rand::{self, Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
#[cfg(feature = "execute")]
use std::collections::BTreeMap;

/// Empties a destination table so a re-run replaces the previous split instead
/// of appending a second copy of it. A destination that was never written has
/// no table yet, and `delete` would fail on the missing table.
#[cfg(feature = "execute")]
async fn clear_destination(db: &CachedDB) -> Result<()> {
    db.ensure_flushed().await?;
    let guard = db.db.read().await;
    if guard.inner().raw().await.is_err() {
        return Ok(());
    }
    guard.delete("true").await
}

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
            "Split a dataset into training and testing subsets, keeping every class at its original proportion in both subsets",
            "AI/ML/Dataset",
        );
        node.set_flowscript_name("ml", "stratifiedSplit");
        node.set_version(1);
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
            "Share of each class that goes to the training set (rest goes to test). Must be between 0 and 1, exclusive",
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
            "seed",
            "Seed",
            "Seed for the per-class shuffle. Any non-zero value makes the split reproducible; 0 draws a fresh seed each run and logs it",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

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
            "Destination database that receives the training rows. It is cleared before every run",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "test",
            "Test Database",
            "Destination database that receives the testing rows. It is cleared before every run",
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
        let ratio: f64 = context.evaluate_pin("split").await?;
        let label_column: String = context.evaluate_pin("label_column").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;

        if !ratio.is_finite() || ratio <= 0.0 || ratio >= 1.0 {
            return Err(flow_like_types::anyhow!(
                "Split Ratio must be between 0 and 1 (exclusive), got {ratio}"
            ));
        }

        let label_column = label_column.trim().to_string();
        if label_column.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Label Column must name the column that is stratified over, but it is empty"
            ));
        }

        if source.cache_key.is_empty() || train.cache_key.is_empty() || test.cache_key.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Data Source, Training Database and Test Database must all be connected to a database"
            ));
        }

        if train.cache_key == test.cache_key {
            return Err(flow_like_types::anyhow!(
                "Training Database and Test Database must be different databases, both resolve to '{}'",
                train.cache_key
            ));
        }

        if source.cache_key == train.cache_key || source.cache_key == test.cache_key {
            context.log_message(
                "The data source is also a split destination and will be replaced by its own split",
                LogLevel::Warn,
            );
        }

        let source_db = source.load(context).await?;
        let test_db = test.load(context).await?;
        let train_db = train.load(context).await?;

        let mut class_buckets: BTreeMap<String, Vec<flow_like_types::Value>> = BTreeMap::new();
        let mut total_rows = 0usize;
        let mut null_labels = 0usize;

        {
            source_db.ensure_flushed().await?;
            let source_guard = source_db.db.read().await;
            let source_table = source_guard.inner().raw().await?;
            let query = source_table.query();
            let mut item_stream = query.execute().await?;

            loop {
                let next = item_stream.try_next().await.map_err(|err| {
                    flow_like_types::anyhow!("Failed to read the source dataset: {err}")
                })?;
                let Some(batch) = next else {
                    break;
                };

                for item in record_batch_to_value(&batch)? {
                    let label = match item.get(&label_column) {
                        Some(flow_like_types::Value::String(label)) => label.clone(),
                        Some(flow_like_types::Value::Null) => {
                            null_labels += 1;
                            "null".to_string()
                        }
                        Some(other) => other.to_string(),
                        // Arrow batches are schema uniform, so a single row
                        // without the column means the dataset has no such column.
                        None => {
                            let columns = item
                                .as_object()
                                .map(|object| object.keys().cloned().collect::<Vec<_>>())
                                .unwrap_or_default();
                            return Err(flow_like_types::anyhow!(
                                "Label Column '{}' does not exist in the source dataset. Available columns: {}",
                                label_column,
                                columns.join(", ")
                            ));
                        }
                    };

                    total_rows += 1;
                    class_buckets.entry(label).or_default().push(item);
                }
            }
        }

        if total_rows == 0 {
            context.log_message(
                "Source dataset is empty, both destination databases are cleared and left empty",
                LogLevel::Warn,
            );
        }

        if null_labels > 0 {
            context.log_message(
                &format!(
                    "{null_labels} rows have a null '{label_column}' value and are stratified as their own class"
                ),
                LogLevel::Warn,
            );
        }

        let seed = if seed == 0 {
            let generated: u64 = rand::rng().random();
            context.log_message(
                &format!(
                    "No seed set, using {generated}. Set the Seed pin to this value to reproduce the split"
                ),
                LogLevel::Info,
            );
            generated
        } else {
            seed as u64
        };

        let class_count = class_buckets.len();
        let mut rng = StdRng::seed_from_u64(seed);
        let mut train_items = Vec::with_capacity(total_rows);
        let mut test_items = Vec::with_capacity(total_rows);
        let mut singleton_classes: Vec<String> = Vec::new();
        let mut rebalanced_classes: Vec<(String, usize, usize)> = Vec::new();

        for (label, mut items) in class_buckets {
            items.shuffle(&mut rng);
            let count = items.len();

            let train_count = if count == 1 {
                singleton_classes.push(label);
                1
            } else {
                // Rounding alone starves tiny classes: 3 rows at a 0.9 ratio would
                // round to 3 train / 0 test and drop the class from the test set.
                let rounded = ((count as f64) * ratio).round() as usize;
                let bounded = rounded.clamp(1, count - 1);
                if bounded != rounded {
                    rebalanced_classes.push((label, count, bounded));
                }
                bounded
            };

            let held_out = items.split_off(train_count);
            train_items.extend(items);
            test_items.extend(held_out);
        }

        for class in singleton_classes {
            context.log_message(
                &format!(
                    "Class '{class}' has a single row and cannot appear in both splits, it was assigned to the training set"
                ),
                LogLevel::Warn,
            );
        }

        for (class, count, train_count) in rebalanced_classes {
            let test_count = count - train_count;
            context.log_message(
                &format!(
                    "Class '{class}' has only {count} rows, the {ratio} ratio was adjusted to {train_count} train / {test_count} test so the class stays in both splits"
                ),
                LogLevel::Warn,
            );
        }

        context.log_message(
            &format!(
                "Stratified split on '{label_column}': {class_count} classes, {} train / {} test out of {total_rows} rows",
                train_items.len(),
                test_items.len()
            ),
            LogLevel::Debug,
        );

        clear_destination(&train_db).await?;
        clear_destination(&test_db).await?;

        if !train_items.is_empty() {
            train_db.insert_from(context, train_items).await?;
        }
        if !test_items.is_empty() {
            test_db.insert_from(context, test_items).await?;
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
