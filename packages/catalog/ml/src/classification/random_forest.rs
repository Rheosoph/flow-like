//! Node for Fitting a Random Forest Classifier
//!
//! Bags a configurable number of [`linfa_trees::DecisionTree`] learners over bootstrapped rows and
//! a random subset of the features, using [`linfa_ensemble::RandomForestParams`].

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, PersistedEnsemble, values_to_array1_target,
    values_to_array2_f64,
};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::{Fit, Predict};
#[cfg(feature = "execute")]
use linfa_ensemble::{EnsembleLearner, RandomForestParams};
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

/// Ensemble size above which fit time and the serialized model start to hurt in a hosted flow.
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
pub struct FitRandomForestNode {}

impl FitRandomForestNode {
    pub fn new() -> Self {
        FitRandomForestNode {}
    }
}

#[async_trait]
impl NodeLogic for FitRandomForestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_random_forest",
            "Train Classifier (Random Forest)",
            "Fit/Train a Random Forest classifier: many Decision Trees, each grown on a bootstrapped sample of the rows and a random subset of the features, combined by majority vote. Far more robust to overfitting than a single tree, at the price of interpretability. Model size and fit time grow linearly with Ensemble Size, so a forest of 500 trees costs roughly 500x a single tree.",
            "AI/ML/Classification",
        );
        node.set_flowscript_name("ml", "fitRandomForest");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(5) // One full tree fit per ensemble member
                .set_governance(5) // A vote across many trees is much harder to audit than one tree
                .set_reliability(8) // Bagging averages away the variance of a single tree
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins Random Forest training",
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
            "ensemble_size",
            "Ensemble Size",
            "Number of Decision Trees to grow. Both fit time and the size of the saved model scale linearly with this value.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 2000.0)).build())
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "bootstrap_proportion",
            "Bootstrap Proportion",
            "Share of the training rows drawn (with replacement) for each tree. Must be greater than 0 and at most 1.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.7)));

        node.add_input_pin(
            "feature_proportion",
            "Feature Proportion",
            "Share of the features offered to each tree. Must be at most 1. Leave at 0 for the textbook default of sqrt(feature count) features per tree.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.0)));

        node.add_input_pin(
            "max_depth",
            "Max Depth",
            "Maximum depth of each tree. 0 or less means unlimited, which grows deeper trees and a larger model.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "min_weight_split",
            "Min Samples Split",
            "Minimum summed sample weight a node needs before it may be split. Without row weights this is simply the minimum number of samples.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((1.0, 100000.0)).build())
        .set_default_value(Some(json!(2.0)));

        node.add_input_pin(
            "seed",
            "Seed",
            "Seed for the bootstrap and feature sampling. Fixing it makes the row and feature draws reproducible. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.",
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
            "Thread-safe handle to the trained Random Forest classifier",
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
        let ensemble_size: i64 = context.evaluate_pin("ensemble_size").await?;
        let bootstrap_proportion: f64 = context.evaluate_pin("bootstrap_proportion").await?;
        let feature_proportion: f64 = context.evaluate_pin("feature_proportion").await?;
        let max_depth: i64 = context.evaluate_pin("max_depth").await?;
        let min_weight_split: f64 = context.evaluate_pin("min_weight_split").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;

        if ensemble_size < 1 {
            return Err(anyhow!(
                "Ensemble Size must be at least 1, got {ensemble_size}"
            ));
        }
        if !bootstrap_proportion.is_finite() || !(0.0..=1.0).contains(&bootstrap_proportion) {
            return Err(anyhow!(
                "Bootstrap Proportion must be a finite value greater than 0 and at most 1, got {bootstrap_proportion}"
            ));
        }
        if bootstrap_proportion <= 0.0 {
            return Err(anyhow!(
                "Bootstrap Proportion must be greater than 0, got {bootstrap_proportion}"
            ));
        }
        if !feature_proportion.is_finite() || !(0.0..=1.0).contains(&feature_proportion) {
            return Err(anyhow!(
                "Feature Proportion must be a finite value between 0 and 1, got {feature_proportion}. Use 0 for the sqrt(feature count) default."
            ));
        }
        if !min_weight_split.is_finite() || min_weight_split <= 0.0 {
            return Err(anyhow!(
                "Min Samples Split must be a finite value greater than 0, got {min_weight_split}"
            ));
        }
        if seed < 0 {
            return Err(anyhow!("Seed must not be negative, got {seed}"));
        }
        if ensemble_size > LARGE_ENSEMBLE_WARNING {
            context.log_message(
                &format!(
                    "Ensemble Size {ensemble_size} means {ensemble_size} full tree fits and {ensemble_size} trees inside the saved model"
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
        // `bootstrap_with_indices` draws with `gen_range(0..nsamples)` and `gen_range(0..nfeatures)`,
        // both of which panic on an empty range.
        if n_samples == 0 {
            return Err(anyhow!(
                "Training set is empty, Random Forest needs at least one row"
            ));
        }
        if n_features == 0 {
            return Err(anyhow!(
                "Training vectors are empty, Random Forest needs at least one feature"
            ));
        }
        if let Some((row, col)) = first_non_finite(&train_array) {
            return Err(anyhow!(
                "Row {row}, feature {col} is not finite (NaN or Inf). Random Forest requires finite features."
            ));
        }

        let class_ids = distinct_classes(&target_array);
        if class_ids.len() < 2 {
            context.log_message(
                &format!(
                    "Target col `{targets_col}` holds a single class: {}. Every tree will vote the same way and the forest cannot discriminate.",
                    describe_classes(&class_ids, classes.as_ref())
                ),
                LogLevel::Warn,
            );
        }

        // sqrt(p) features per tree is the standard Random Forest default and cannot be expressed
        // as a fixed proportion, so 0 selects it once the feature count is known.
        let feature_proportion = if feature_proportion <= 0.0 {
            let auto = (n_features as f64).sqrt() / n_features as f64;
            context.log_message(
                &format!(
                    "Feature Proportion auto-selected as {auto:.4} (sqrt of {n_features} features)"
                ),
                LogLevel::Debug,
            );
            auto.clamp(f64::MIN_POSITIVE, 1.0)
        } else {
            feature_proportion
        };

        context.log_message(
            &format!(
                "Training {ensemble_size} trees on {n_samples} rows, {n_features} features, {} classes: {}",
                class_ids.len(),
                describe_classes(&class_ids, classes.as_ref())
            ),
            LogLevel::Debug,
        );

        let ds = DatasetBase::from(train_array).with_targets(target_array);

        let mut tree_params = LinfaDecisionTree::<f64, usize>::params();
        if max_depth > 0 {
            tree_params = tree_params.max_depth(Some(max_depth as usize));
        }
        tree_params = tree_params.min_weight_split(min_weight_split as f32);

        let t0 = std::time::Instant::now();
        let forest: EnsembleLearner<LinfaDecisionTree<f64, usize>> =
            RandomForestParams::new_fixed_rng(
                tree_params,
                Xoshiro256Plus::seed_from_u64(seed as u64),
            )
            .ensemble_size(ensemble_size as usize)
            .bootstrap_proportion(bootstrap_proportion)
            .feature_proportion(feature_proportion)
            .fit(&ds)
            .map_err(|err| anyhow!("Random Forest fit failed: {err}"))?;
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        let grown = forest.models.len();
        if grown != ensemble_size as usize {
            context.log_message(
                &format!("Requested {ensemble_size} trees but the ensemble kept {grown}"),
                LogLevel::Warn,
            );
        }

        // A forest that cannot beat the majority-class baseline on its own training data is
        // degenerate; the depth or the feature proportion is usually too small.
        let predictions: Array1<usize> = forest.predict(ds.records());
        let correct = predictions
            .iter()
            .zip(ds.targets().iter())
            .filter(|(predicted, actual)| predicted == actual)
            .count();
        let accuracy = correct as f64 / n_samples as f64;
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for id in ds.targets().iter() {
            *counts.entry(*id).or_insert(0) += 1;
        }
        let baseline =
            counts.values().copied().max().unwrap_or(n_samples) as f64 / n_samples as f64;
        if accuracy <= baseline {
            context.log_message(
                &format!(
                    "Training accuracy {accuracy:.4} did not beat the majority-class baseline {baseline:.4}. Raise Max Depth, raise Feature Proportion, or grow more trees."
                ),
                LogLevel::Warn,
            );
        } else {
            context.log_message(
                &format!(
                    "Grew {grown} trees, training accuracy: {accuracy:.4} (baseline {baseline:.4})"
                ),
                LogLevel::Debug,
            );
        }

        let model = MLModel::RandomForest(ModelWithMeta {
            model: PersistedEnsemble(forest),
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
