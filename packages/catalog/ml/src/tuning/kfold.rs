//! K-Fold Cross Validation Dataset Generator
//!
//! Generates K train/test fold pairs for cross-validation evaluation and drives the
//! connected fold branch once per fold, synchronously, like the `For Each` control node.

#[cfg(feature = "execute")]
use ahash::AHashSet;
#[cfg(feature = "execute")]
use flow_like::flow::execution::internal_node::InternalNode;
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
use flow_like_types::rand::{self, seq::SliceRandom};
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Output schema for K-Fold generator
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KFoldInfo {
    /// Number of folds generated
    pub k: usize,
    /// Total number of samples in dataset
    pub total_samples: usize,
    /// Approximate samples per fold
    pub samples_per_fold: usize,
}

/// Empties a fold database before it is refilled, so re-running a fold replaces its rows instead
/// of appending a second copy. A destination that was never written has no table yet, and `delete`
/// would fail on the missing table — which is the normal state on the very first fold.
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
pub struct KFoldGeneratorNode {}

impl KFoldGeneratorNode {
    pub fn new() -> Self {
        KFoldGeneratorNode {}
    }
}

#[async_trait]
impl NodeLogic for KFoldGeneratorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_dataset_kfold",
            "K-Fold Split",
            "Generate K train/test splits for cross-validation. Each fold uses (K-1)/K data for training and 1/K for validation, and runs the connected fold branch once per fold.",
            "AI/ML/Dataset",
        );
        node.set_flowscript_name("ml", "kfold");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "k",
            "K (Folds)",
            "Number of folds for cross-validation (typically 5 or 10)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "shuffle",
            "Shuffle",
            "Randomly shuffle data before splitting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "source",
            "Source Database",
            "Source database containing the dataset. It is only read, never modified.",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // K pairs of train/test databases
        node.add_input_pin(
            "train_db",
            "Training Database",
            "Database to receive training data for each fold (will be cleared and filled K times)",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "test_db",
            "Validation Database",
            "Database to receive validation data for each fold (will be cleared and filled K times)",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_fold",
            "For Each Fold",
            "Executed K times, once per fold, and awaited before the next fold overwrites the databases. Connect your training/evaluation logic here.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_done",
            "Done",
            "Triggered after all folds completed successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "fold_index",
            "Current Fold",
            "Current fold index (0 to K-1)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "info",
            "Fold Info",
            "Information about the K-fold split",
            VariableType::Struct,
        )
        .set_schema::<KFoldInfo>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        let exec_fold = context.get_pin_by_name("exec_fold").await?;
        let exec_done = context.get_pin_by_name("exec_done").await?;
        context.deactivate_exec_pin_ref(&exec_fold).await?;
        context.deactivate_exec_pin_ref(&exec_done).await?;

        let k: i64 = context.evaluate_pin("k").await?;
        let shuffle: bool = context.evaluate_pin("shuffle").await?;
        let source_ref: NodeDBConnection = context.evaluate_pin("source").await?;
        let train_db_ref: NodeDBConnection = context.evaluate_pin("train_db").await?;
        let test_db_ref: NodeDBConnection = context.evaluate_pin("test_db").await?;

        // Checked before the cast: a negative i64 would wrap into a huge usize.
        if k < 2 {
            return Err(flow_like_types::anyhow!(
                "K must be at least 2 for cross-validation, got {}",
                k
            ));
        }
        let k = k as usize;

        if train_db_ref.cache_key == test_db_ref.cache_key {
            return Err(flow_like_types::anyhow!(
                "Training and validation database must be different, both point at `{}`",
                train_db_ref.cache_key
            ));
        }
        if source_ref.cache_key == train_db_ref.cache_key
            || source_ref.cache_key == test_db_ref.cache_key
        {
            return Err(flow_like_types::anyhow!(
                "Source database `{}` must differ from the fold databases; they are cleared on every fold and the source rows would be lost",
                source_ref.cache_key
            ));
        }

        // Fully materialize the source and release its lock before any fold body runs,
        // otherwise a fold body writing to a database would contend with this read guard.
        let mut all_items = {
            let source = source_ref.load(context).await?;
            source.ensure_flushed().await?;
            let source_table = {
                let source_guard = source.db.read().await;
                source_guard.inner().raw().await?
            };
            let query = source_table.query();
            let mut item_stream = query.execute().await?;

            let mut items: Vec<flow_like_types::Value> = Vec::new();
            loop {
                match item_stream.try_next().await {
                    Ok(Some(batch)) => items.extend(record_batch_to_value(&batch)?),
                    Ok(None) => break,
                    Err(err) => {
                        return Err(flow_like_types::anyhow!(
                            "Failed to read source dataset for K-Fold split: {}",
                            err
                        ));
                    }
                }
            }
            items
        };

        let total_samples = all_items.len();
        if total_samples < k {
            return Err(flow_like_types::anyhow!(
                "Not enough samples ({}) for {} folds",
                total_samples,
                k
            ));
        }

        if shuffle {
            let mut rng = rand::rng();
            all_items.shuffle(&mut rng);
        }

        let fold_size = total_samples / k;
        let remainder = total_samples % k;

        context.log_message(
            &format!(
                "K-Fold CV: {} samples, {} folds, ~{} per fold",
                total_samples, k, fold_size
            ),
            LogLevel::Info,
        );

        let info = KFoldInfo {
            k,
            total_samples,
            samples_per_fold: fold_size,
        };
        context.set_pin_value("info", json!(info)).await?;

        let connected = exec_fold.get_connected_nodes();
        if connected.is_empty() {
            context.log_message(
                "K-Fold: nothing connected to `For Each Fold`, the folds are generated but never evaluated",
                LogLevel::Warn,
            );
        }

        // Seeded with this node's id so a fold body looping back into the K-Fold node
        // cannot re-enter it and clobber the databases of the running fold.
        let node_id = context.read_node().await.id.clone();
        let recursion_guard = AHashSet::from_iter(vec![node_id]);

        context.activate_exec_pin_ref(&exec_fold).await?;

        for fold_idx in 0..k {
            let val_start = fold_idx * fold_size + fold_idx.min(remainder);
            let val_end = val_start + fold_size + if fold_idx < remainder { 1 } else { 0 };

            let mut train_items = Vec::with_capacity(total_samples - (val_end - val_start));
            let mut val_items = Vec::with_capacity(val_end - val_start);

            for (i, item) in all_items.iter().enumerate() {
                if i >= val_start && i < val_end {
                    val_items.push(item.clone());
                } else {
                    train_items.push(item.clone());
                }
            }

            context.log_message(
                &format!(
                    "Fold {}/{}: {} train, {} validation",
                    fold_idx + 1,
                    k,
                    train_items.len(),
                    val_items.len()
                ),
                LogLevel::Debug,
            );

            // Both databases are flushed after filling: inserts are buffered, and the fold
            // body reads them within this same tick.
            let train_db = train_db_ref.load(context).await?;
            clear_destination(&train_db).await?;
            if !train_items.is_empty() {
                train_db.insert_from(context, train_items).await?;
            }
            train_db.ensure_flushed().await?;

            let test_db = test_db_ref.load(context).await?;
            clear_destination(&test_db).await?;
            if !val_items.is_empty() {
                test_db.insert_from(context, val_items).await?;
            }
            test_db.ensure_flushed().await?;

            context
                .set_pin_value("fold_index", json!(fold_idx as i64))
                .await?;

            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;
                let run = InternalNode::trigger(
                    &mut sub_context,
                    &mut Some(recursion_guard.clone()),
                    true,
                )
                .await;
                sub_context.end_trace();
                context.push_sub_context(&mut sub_context);

                if let Err(error) = run {
                    context.log_message(
                        &format!("Error: {:?} in fold {}/{}", error, fold_idx + 1, k),
                        LogLevel::Error,
                    );
                    context.deactivate_exec_pin_ref(&exec_fold).await?;
                    return Err(flow_like_types::anyhow!(
                        "K-Fold cross-validation aborted, fold {} of {} failed: {:?}",
                        fold_idx + 1,
                        k,
                        error
                    ));
                }
            }
        }

        context.deactivate_exec_pin_ref(&exec_fold).await?;
        context.activate_exec_pin_ref(&exec_done).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }
}
