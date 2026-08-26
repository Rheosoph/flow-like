//! Node for Fitting an AdaBoost Classifier
//!
//! Boosts a sequence of shallow [`linfa_trees::DecisionTree`] learners with the multi-class SAMME
//! algorithm from [`linfa_ensemble::AdaBoostParams`]. Unlike the bagging in Random Forest, each
//! learner is fit on the residual mistakes of the ones before it.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, PersistedAdaBoost, values_to_array1_target,
    values_to_array2_f64,
};
#[cfg(feature = "execute")]
use flow_like::flow::{board::Board, execution::LogLevel};
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
#[cfg(feature = "execute")]
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_ensemble::AdaBoostParams;
#[cfg(feature = "execute")]
use linfa_trees::DecisionTree as LinfaDecisionTree;
#[cfg(feature = "execute")]
use ndarray::{Array1, Array2};
// linfa 0.8 is built against rand 0.8, so the RNG handed to it cannot come from
// `flow_like_types::rand` (0.9) — the `Rng` traits are different types.
#[cfg(feature = "execute")]
use rand_xoshiro::Xoshiro256Plus;
#[cfg(feature = "execute")]
use rand_xoshiro::rand_core::SeedableRng;
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

/// Estimator count above which fit time and the serialized model start to hurt in a hosted flow.
#[cfg(feature = "execute")]
const LARGE_ENSEMBLE_WARNING: i64 = 500;

/// Position of the first non-finite feature, if any.
///
/// linfa-trees sorts every feature column with `partial_cmp(..).unwrap_or(Greater)`, which is not a
/// total order once a NaN is present; modern `slice::sort_by` detects that and panics. Rejecting
/// the input here turns that panic into a message that names the offending cell.
#[cfg(feature = "execute")]
fn first_non_finite(records: &Array2<f64>) -> Option<(usize, usize)> {
    records
        .indexed_iter()
        .find(|(_, value)| !value.is_finite())
        .map(|((row, col), _)| (row, col))
}

/// Distinct class ids in ascending order.
#[cfg(feature = "execute")]
fn distinct_classes(targets: &Array1<usize>) -> Vec<usize> {
    let mut ids: Vec<usize> = targets
        .iter()
        .copied()
        .collect::<HashSet<usize>>()
        .into_iter()
        .collect();
    ids.sort_unstable();
    ids
}

/// Renders class ids with their names when the target column was categorical.
#[cfg(feature = "execute")]
fn describe_classes(ids: &[usize], classes: Option<&HashMap<usize, String>>) -> String {
    ids.iter()
        .map(|id| match classes.and_then(|map| map.get(id)) {
            Some(name) => format!("`{name}` ({id})"),
            None => id.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[crate::register_node]
#[derive(Default)]
pub struct FitAdaBoostNode {}

impl FitAdaBoostNode {
    pub fn new() -> Self {
        FitAdaBoostNode {}
    }
}

#[async_trait]
impl NodeLogic for FitAdaBoostNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_adaboost",
            "Train Classifier (AdaBoost)",
            "Fit/Train an AdaBoost classifier using multi-class SAMME boosting over shallow Decision Trees. Each learner focuses on the rows its predecessors got wrong, so boosting usually beats a single tree on weak signal, but it is far more sensitive to label noise and outliers than Random Forest. Estimators is a maximum, not a guarantee: boosting stops early once a learner is no better than random guessing.",
            "AI/ML/Classification",
        );
        node.set_flowscript_name("ml", "fitAdaboost");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(5) // One sequential tree fit per estimator, no parallelism
                .set_governance(5) // A weighted vote over many stumps is hard to audit
                .set_reliability(6) // Strong on weak signal, but noise-sensitive
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins AdaBoost training",
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
            "n_estimators",
            "Estimators",
            "Maximum number of boosting rounds. Boosting stops early once a learner performs no better than random guessing, so the fitted model may hold fewer estimators than requested.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 2000.0)).build())
        .set_default_value(Some(json!(50)));

        node.add_input_pin(
            "learning_rate",
            "Learning Rate",
            "Shrinkage applied to each learner's vote. Must be positive. Values below 1 regularize the ensemble but need more estimators; 0.1 with 500 estimators is a common pairing.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.001, 2.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "max_depth",
            "Base Tree Depth",
            "Depth of each weak learner. AdaBoost is designed around shallow trees; 1 gives classic decision stumps. Deep base trees defeat the point of boosting and overfit quickly.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 32.0)).build())
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "seed",
            "Seed",
            "Seed for the base learner sampling. Fixing it makes the sampling reproducible. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0.0, 4294967295.0)).build())
        .set_default_value(Some(json!(42)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained AdaBoost classifier",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "estimators_kept",
            "Estimators Kept",
            "Number of estimators actually retained after early stopping, which may be lower than the requested maximum",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let n_estimators: i64 = context.evaluate_pin("n_estimators").await?;
        let learning_rate: f64 = context.evaluate_pin("learning_rate").await?;
        let max_depth: i64 = context.evaluate_pin("max_depth").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;

        if n_estimators < 1 {
            return Err(anyhow!("Estimators must be at least 1, got {n_estimators}"));
        }
        if !learning_rate.is_finite() || learning_rate <= 0.0 {
            return Err(anyhow!(
                "Learning Rate must be a finite value greater than 0, got {learning_rate}"
            ));
        }
        if max_depth < 1 {
            return Err(anyhow!(
                "Base Tree Depth must be at least 1, got {max_depth}. AdaBoost needs a real weak learner."
            ));
        }
        if seed < 0 {
            return Err(anyhow!("Seed must not be negative, got {seed}"));
        }
        if n_estimators > LARGE_ENSEMBLE_WARNING {
            context.log_message(
                &format!(
                    "Estimators {n_estimators} means up to {n_estimators} sequential tree fits; boosting cannot parallelize them"
                ),
                LogLevel::Warn,
            );
        }

        let t0 = std::time::Instant::now();
        let (train_array, target_array, classes, targets_col) = match source.as_str() {
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
                if records.is_empty() {
                    return Err(anyhow!(
                        "Database returned no rows, there is nothing to train on"
                    ));
                }
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records, &targets_col)?;
                (train_array, target_array, classes, targets_col)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let (n_samples, n_features) = train_array.dim();
        if n_samples == 0 {
            return Err(anyhow!(
                "Training set is empty, AdaBoost needs at least one row"
            ));
        }
        if n_features == 0 {
            return Err(anyhow!(
                "Training vectors are empty, AdaBoost needs at least one feature"
            ));
        }
        if let Some((row, col)) = first_non_finite(&train_array) {
            return Err(anyhow!(
                "Row {row}, feature {col} is not finite (NaN or Inf). AdaBoost requires finite features."
            ));
        }

        // Upstream returns a terse `AdaBoost requires at least 2 classes`; naming the column and the
        // class that was actually found is far more actionable.
        let class_ids = distinct_classes(&target_array);
        if class_ids.len() < 2 {
            return Err(anyhow!(
                "AdaBoost needs at least 2 classes, but target col `{targets_col}` holds only {}. Check the target column or widen the training set.",
                describe_classes(&class_ids, classes.as_ref())
            ));
        }

        context.log_message(
            &format!(
                "Boosting up to {n_estimators} estimators on {n_samples} rows, {n_features} features, {} classes: {}",
                class_ids.len(),
                describe_classes(&class_ids, classes.as_ref())
            ),
            LogLevel::Debug,
        );

        let ds = DatasetBase::from(train_array).with_targets(target_array);

        let tree_params =
            LinfaDecisionTree::<f64, usize>::params().max_depth(Some(max_depth as usize));

        let t0 = std::time::Instant::now();
        let boosted =
            AdaBoostParams::new_fixed_rng(tree_params, Xoshiro256Plus::seed_from_u64(seed as u64))
                .n_estimators(n_estimators as usize)
                .learning_rate(learning_rate)
                .fit(&ds)
                .map_err(|err| anyhow!("AdaBoost fit failed: {err}"))?;
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        // Early stopping is the normal outcome, not a failure, so report what was actually kept
        // rather than letting the requested maximum imply a count that was never reached.
        let kept = boosted.n_estimators();
        if kept < n_estimators as usize {
            context.log_message(
                &format!(
                    "Boosting stopped after {kept} of {n_estimators} estimators; the next learner was no better than random guessing"
                ),
                LogLevel::Debug,
            );
        }

        let model = MLModel::AdaBoost(ModelWithMeta {
            model: PersistedAdaBoost(boosted),
            classes,
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("model", json!(node_model)).await?;
        context
            .set_pin_value("estimators_kept", json!(kept as i64))
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

    #[cfg(feature = "execute")]
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
