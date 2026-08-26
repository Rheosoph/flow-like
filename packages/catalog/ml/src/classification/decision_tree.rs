//! Node for Fitting Decision Tree Classifier
//!
//! This node loads a dataset, transforms it into a classification dataset,
//! and fits a Decision Tree model using the [`linfa_trees`] crate.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array1_target,
    values_to_array2_f64,
};
use flow_like::flow::board::Board;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_trees::{DecisionTree as LinfaDecisionTree, SplitQuality};
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct FitDecisionTreeNode {}

impl FitDecisionTreeNode {
    pub fn new() -> Self {
        FitDecisionTreeNode {}
    }
}

#[async_trait]
impl NodeLogic for FitDecisionTreeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_decision_tree",
            "Train Classifier (Decision Tree)",
            "Fit/Train a Decision Tree classifier. Native multi-class support with interpretable rules.",
            "AI/ML/Classification",
        );
        node.set_flowscript_name("ml", "fitDecisionTree");
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(8)
                .set_governance(7) // More interpretable than SVM
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins Decision Tree training",
            VariableType::Execution,
        );

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose which backend supplies the training data",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "max_depth",
            "Max Depth",
            "Maximum depth of the tree. None means unlimited.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "min_samples_split",
            "Min Samples Split",
            "Minimum number of samples required to split a node",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_input_pin(
            "split_quality",
            "Split Quality",
            "Impurity metric that scores candidate splits. Gini is cheaper, Entropy favours balanced information gain.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Gini".to_string(), "Entropy".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Gini")));

        node.add_input_pin(
            "min_weight_leaf",
            "Min Samples Leaf",
            "Minimum number of samples (total sample weight) a split has to place in each leaf",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 10000.)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "min_impurity_decrease",
            "Min Impurity Decrease",
            "Minimum impurity decrease a split has to bring to be applied. Must be greater than zero; larger values prune harder.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.00001)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained Decision Tree classifier",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let max_depth: i64 = context.evaluate_pin("max_depth").await?;
        let min_samples_split: i64 = context.evaluate_pin("min_samples_split").await?;
        // Boards placed before these pins existed fall back to the linfa defaults, which is exactly
        // what the node used to pass.
        let split_quality: String = context
            .evaluate_pin("split_quality")
            .await
            .unwrap_or_else(|_| "Gini".to_string());
        let min_weight_leaf: f64 = context.evaluate_pin("min_weight_leaf").await.unwrap_or(1.0);
        let min_impurity_decrease: f64 = context
            .evaluate_pin("min_impurity_decrease")
            .await
            .unwrap_or(0.00001);

        let split_quality = match split_quality.as_str() {
            "Gini" => SplitQuality::Gini,
            "Entropy" => SplitQuality::Entropy,
            other => {
                return Err(anyhow!(
                    "Unknown split quality `{other}`, expected `Gini` or `Entropy`"
                ));
            }
        };
        if !min_weight_leaf.is_finite() || min_weight_leaf < 0.0 {
            return Err(anyhow!(
                "Min Samples Leaf has to be a finite value >= 0, got {min_weight_leaf}"
            ));
        }
        // linfa rejects anything below f64::EPSILON, and a NaN would slip past its `<` check and
        // silently disable the impurity criterion altogether.
        if !min_impurity_decrease.is_finite() || min_impurity_decrease < f64::EPSILON {
            return Err(anyhow!(
                "Min Impurity Decrease has to be greater than {:e}, got {}",
                f64::EPSILON,
                min_impurity_decrease
            ));
        }

        let t0 = std::time::Instant::now();
        let (ds, classes) = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let targets_col: String = context.evaluate_pin("targets").await?;

                let records = {
                    let cached_db = database.load(context).await?;
                    cached_db.ensure_flushed().await?;
                    let database = cached_db.db.read().await;
                    let schema = database.schema().await?;
                    let existing_cols: HashSet<String> =
                        schema.fields.iter().map(|f| f.name().clone()).collect();
                    if !existing_cols.contains(&records_col) {
                        return Err(anyhow!(format!(
                            "Database doesn't contain train col `{}`!",
                            records_col
                        )));
                    }
                    if !existing_cols.contains(&targets_col) {
                        return Err(anyhow!(format!(
                            "Database doesn't contain target col `{}`!",
                            targets_col
                        )));
                    }
                    database
                        .filter(
                            "true",
                            Some(vec![records_col.to_string(), targets_col.to_string()]),
                            MAX_ML_PREDICTION_RECORDS,
                            0,
                        )
                        .await?
                };
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );
                // linfa unwraps the modal class of the root node, which panics on an empty dataset.
                if records.is_empty() {
                    return Err(anyhow!(
                        "No training records in the database; Decision Tree fitting needs at least one row"
                    ));
                }

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records, &targets_col)?;
                (
                    DatasetBase::from(train_array).with_targets(target_array),
                    classes,
                )
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let t0 = std::time::Instant::now();
        let mut params = LinfaDecisionTree::params()
            .split_quality(split_quality)
            .min_weight_leaf(min_weight_leaf as f32)
            .min_impurity_decrease(min_impurity_decrease);
        if max_depth > 0 {
            params = params.max_depth(Some(max_depth as usize));
        }
        if min_samples_split > 0 {
            params = params.min_weight_split(min_samples_split as f32);
        }
        let tree_model = params.fit(&ds)?;
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        let num_leaves = tree_model.num_leaves();
        context.log_message(
            &format!(
                "Fitted tree with {} leaves and depth {}",
                num_leaves,
                tree_model.max_depth()
            ),
            LogLevel::Debug,
        );
        if num_leaves <= 1 {
            context.log_message(
                "Decision tree collapsed to a single leaf and predicts one class for every input. Lower Min Impurity Decrease / Min Samples Split or raise Max Depth.",
                LogLevel::Warn,
            );
        }

        let model = MLModel::DecisionTree(ModelWithMeta {
            model: tree_model,
            classes,
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("model", json!(node_model)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        use flow_like_catalog_core::NodeDBConnection;

        let source_pin: String = node
            .get_pin_by_name("source")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        if source_pin == *"Database" {
            if node.get_pin_by_name("database").is_none() {
                node.add_input_pin(
                    "database",
                    "Database",
                    "Database Connection",
                    VariableType::Struct,
                )
                .set_schema::<NodeDBConnection>()
                .set_options(PinOptions::new().set_enforce_schema(true).build());
            }
            if node.get_pin_by_name("records").is_none() {
                node.add_input_pin(
                    "records",
                    "Train Col",
                    "Column Containing the Values to Train on",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("targets").is_none() {
                node.add_input_pin(
                    "targets",
                    "Target Col",
                    "Column Containing the Target Values to Fit the Classifier on",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
