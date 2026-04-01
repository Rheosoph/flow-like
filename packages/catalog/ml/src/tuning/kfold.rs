//! K-Fold Cross Validation Dataset Generator
//!
//! Generates K train/test fold pairs for cross-validation evaluation.

use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
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
            "Generate K train/test splits for cross-validation. Each fold uses (K-1)/K data for training and 1/K for validation.",
            "AI/ML/Dataset",
        );
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
            "Source database containing the dataset",
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
            "Triggered K times, once per fold. Connect your training/evaluation logic here.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_done",
            "Done",
            "Triggered after all folds complete",
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
        context.deactivate_exec_pin("exec_fold").await?;
        context.deactivate_exec_pin("exec_done").await?;

        let k: i64 = context.evaluate_pin("k").await?;
        let shuffle: bool = context.evaluate_pin("shuffle").await?;
        let source: NodeDBConnection = context.evaluate_pin("source").await?;
        let train_db_ref: NodeDBConnection = context.evaluate_pin("train_db").await?;
        let test_db_ref: NodeDBConnection = context.evaluate_pin("test_db").await?;

        let k = k as usize;
        if k < 2 {
            return Err(flow_like_types::anyhow!(
                "K must be at least 2 for cross-validation"
            ));
        }

        // Load all data from source
        let source = source.load(context).await?;
        source.ensure_flushed().await?;
        let source_guard = source.db.read().await;
        let source_table = source_guard.inner().raw().await?;
        let query = source_table.query();
        let mut item_stream = query.execute().await?;

        let mut all_items = Vec::new();
        while let Ok(Some(batch)) = item_stream.try_next().await {
            let items = record_batch_to_value(&batch)?;
            all_items.extend(items);
        }

        let total_samples = all_items.len();
        if total_samples < k {
            return Err(flow_like_types::anyhow!(
                "Not enough samples ({}) for {} folds",
                total_samples,
                k
            ));
        }

        // Shuffle if requested
        if shuffle {
            let mut rng = rand::rng();
            all_items.shuffle(&mut rng);
        }

        // Calculate fold sizes
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

        // Generate each fold
        for fold_idx in 0..k {
            // Calculate validation set range for this fold
            let val_start = fold_idx * fold_size + fold_idx.min(remainder);
            let val_end = val_start + fold_size + if fold_idx < remainder { 1 } else { 0 };

            // Split into train and validation
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

            // Clear and fill train database
            let train_db = train_db_ref.load(context).await?;
            {
                let mut train_db_write = train_db.db.write().await;
                train_db_write.delete("true").await?;
                if !train_items.is_empty() {
                    train_db_write.insert(train_items).await?;
                }
            }

            // Clear and fill validation database
            let test_db = test_db_ref.load(context).await?;
            {
                let mut test_db_write = test_db.db.write().await;
                test_db_write.delete("true").await?;
                if !val_items.is_empty() {
                    test_db_write.insert(val_items).await?;
                }
            }

            // Output current fold index and trigger fold execution
            context
                .set_pin_value("fold_index", json!(fold_idx as i64))
                .await?;
            context.activate_exec_pin("exec_fold").await?;

            // Note: In a real flow, the downstream nodes would execute here
            // This is a loop node pattern - the flow engine handles iteration
        }

        context.deactivate_exec_pin("exec_fold").await?;
        context.activate_exec_pin("exec_done").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }
}
