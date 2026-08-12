//! Node for Fitting Logistic Regression Classifiers
//!
//! Covers both the binary solver ([`linfa_logistic::LogisticRegression`]) and the multinomial
//! solver ([`linfa_logistic::MultiLogisticRegression`]) behind a single "Mode" dropdown.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array1_target,
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
use linfa_logistic::{
    FittedLogisticRegression, LogisticRegression, MultiFittedLogisticRegression,
    MultiLogisticRegression,
};
#[cfg(feature = "execute")]
use ndarray::{Array1, Array2};
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

/// Largest absolute feature value that still counts as "roughly scaled". Above it the LBFGS
/// solver of linfa-logistic regularly stalls, so the node warns and points at the scaler.
#[cfg(feature = "execute")]
const UNSCALED_FEATURE_WARNING: f64 = 100.0;

/// Class ids paired with their sample count, ascending by class id.
#[cfg(feature = "execute")]
fn class_counts(targets: &Array1<usize>) -> Vec<(usize, usize)> {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for id in targets.iter() {
        *counts.entry(*id).or_insert(0) += 1;
    }
    let mut counts: Vec<(usize, usize)> = counts.into_iter().collect();
    counts.sort_unstable();
    counts
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

/// Position of the first non-finite feature, if any. linfa reports these as a bare
/// `InvalidValues` without telling the user where they are.
#[cfg(feature = "execute")]
fn first_non_finite(records: &Array2<f64>) -> Option<(usize, usize)> {
    records
        .indexed_iter()
        .find(|(_, value)| !value.is_finite())
        .map(|((row, col), _)| (row, col))
}

#[crate::register_node]
#[derive(Default)]
pub struct FitLogisticRegressionNode {}

impl FitLogisticRegressionNode {
    pub fn new() -> Self {
        FitLogisticRegressionNode {}
    }
}

#[async_trait]
impl NodeLogic for FitLogisticRegressionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_logistic_regression",
            "Train Classifier (Logistic Regression)",
            "Fit/Train a Logistic Regression classifier with L2 regularization. Handles binary and multi-class targets and yields interpretable coefficients plus calibrated probabilities. The solver expects features on a comparable scale - fit a Feature Scaler first if your columns have very different ranges.",
            "AI/ML/Classification",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(8)
                .set_governance(8) // Linear coefficients are directly auditable
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins Logistic Regression training",
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
            "mode",
            "Mode",
            "Auto picks the binary solver for two classes and the multinomial (softmax) solver for more. Binary and Multinomial force one of them.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Auto".to_string(),
                    "Binary".to_string(),
                    "Multinomial".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Auto")));

        node.add_input_pin(
            "alpha",
            "Alpha (L2)",
            "Weight of the L2 penalty on the coefficients. 0 disables regularization, larger values shrink the model harder.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "fit_intercept",
            "Fit Intercept",
            "Fit a bias term. Disable only when the features are already centered.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "max_iterations",
            "Max Iterations",
            "Upper bound on LBFGS iterations. Raise it when training accuracy stays at the baseline.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 100000.0)).build())
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "gradient_tolerance",
            "Gradient Tolerance",
            "Smallest gradient norm that still continues the solver. Smaller means a tighter fit and more iterations.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0001)));

        node.add_input_pin(
            "threshold",
            "Threshold",
            "Probability above which linfa's positive class is predicted. You do not choose that class: linfa assigns it to whichever label sorts second, which for a typical imbalanced dataset is the majority class. Raising the threshold therefore makes the OTHER class — usually the rare one — more likely to be predicted. The class the threshold actually governs is logged when training runs. Binary mode only, ignored for multinomial targets.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.5)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained Logistic Regression classifier",
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
        let mode: String = context.evaluate_pin("mode").await?;
        let alpha: f64 = context.evaluate_pin("alpha").await?;
        let fit_intercept: bool = context.evaluate_pin("fit_intercept").await?;
        let max_iterations: i64 = context.evaluate_pin("max_iterations").await?;
        let gradient_tolerance: f64 = context.evaluate_pin("gradient_tolerance").await?;
        let threshold: f64 = context.evaluate_pin("threshold").await?;

        if !alpha.is_finite() || alpha < 0.0 {
            return Err(anyhow!(
                "Alpha must be a finite value >= 0, got {alpha}. Use 0 to train without regularization."
            ));
        }
        if !gradient_tolerance.is_finite() || gradient_tolerance <= 0.0 {
            return Err(anyhow!(
                "Gradient Tolerance must be a finite value > 0, got {gradient_tolerance}"
            ));
        }
        if max_iterations < 1 {
            return Err(anyhow!(
                "Max Iterations must be at least 1, got {max_iterations}"
            ));
        }
        // `FittedLogisticRegression::set_threshold` panics outside [0, 1] instead of erroring.
        if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
            return Err(anyhow!(
                "Threshold must be a finite value between 0.0 and 1.0, got {threshold}"
            ));
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
        if n_features == 0 {
            return Err(anyhow!(
                "Training vectors are empty, Logistic Regression needs at least one feature"
            ));
        }
        if let Some((row, col)) = first_non_finite(&train_array) {
            return Err(anyhow!(
                "Row {row}, feature {col} is not finite (NaN or Inf). Logistic Regression requires finite features."
            ));
        }

        let counts = class_counts(&target_array);
        let class_ids: Vec<usize> = counts.iter().map(|(id, _)| *id).collect();
        if class_ids.len() < 2 {
            return Err(anyhow!(
                "Target col `{targets_col}` holds {} distinct class(es): {}. Logistic Regression needs at least 2.",
                class_ids.len(),
                describe_classes(&class_ids, classes.as_ref())
            ));
        }
        context.log_message(
            &format!(
                "Training on {n_samples} rows, {n_features} features, {} classes: {}",
                class_ids.len(),
                describe_classes(&class_ids, classes.as_ref())
            ),
            LogLevel::Debug,
        );

        let multinomial = match mode.as_str() {
            "Auto" => class_ids.len() > 2,
            "Multinomial" => true,
            "Binary" => {
                if class_ids.len() != 2 {
                    return Err(anyhow!(
                        "Binary mode needs exactly 2 distinct classes in `{targets_col}`, found {}: {}. Switch Mode to Multinomial or Auto.",
                        class_ids.len(),
                        describe_classes(&class_ids, classes.as_ref())
                    ));
                }
                false
            }
            other => {
                return Err(anyhow!(
                    "Unknown Mode `{other}`, expected Auto, Binary or Multinomial"
                ));
            }
        };

        let max_abs = train_array
            .iter()
            .fold(0.0f64, |acc, value| acc.max(value.abs()));
        if max_abs > UNSCALED_FEATURE_WARNING {
            context.log_message(
                &format!(
                    "Largest absolute feature value is {max_abs:.3}. Logistic Regression converges poorly on unscaled features, consider a Feature Scaler node before training."
                ),
                LogLevel::Warn,
            );
        }

        let ds = DatasetBase::from(train_array).with_targets(target_array);

        let t0 = std::time::Instant::now();
        let (model, predictions): (MLModel, Array1<usize>) = if multinomial {
            if threshold != 0.5 {
                context.log_message(
                    "Threshold applies to binary Logistic Regression only and is ignored for multinomial targets",
                    LogLevel::Warn,
                );
            }
            let fitted: MultiFittedLogisticRegression<f64, usize> =
                MultiLogisticRegression::<f64>::default()
                    .alpha(alpha)
                    .with_intercept(fit_intercept)
                    .max_iterations(max_iterations as u64)
                    .gradient_tolerance(gradient_tolerance)
                    .fit(&ds)
                    .map_err(|err| anyhow!("Multinomial Logistic Regression fit failed: {err}"))?;
            let predictions: Array1<usize> = fitted.predict(ds.records());
            (
                MLModel::MultinomialLogisticRegression(ModelWithMeta {
                    model: fitted,
                    classes,
                }),
                predictions,
            )
        } else {
            let fitted: FittedLogisticRegression<f64, usize> = LogisticRegression::<f64>::default()
                .alpha(alpha)
                .with_intercept(fit_intercept)
                .max_iterations(max_iterations as u64)
                .gradient_tolerance(gradient_tolerance)
                .fit(&ds)
                .map_err(|err| anyhow!("Binary Logistic Regression fit failed: {err}"))?;
            // Which label linfa treats as positive is decided by its own ordering, not by the
            // user, so name it explicitly — otherwise a raised threshold appears to push
            // predictions toward the wrong class.
            let positive = fitted.labels().pos.class;
            let positive_name = classes
                .as_ref()
                .and_then(|map| map.get(&positive).cloned())
                .unwrap_or_else(|| positive.to_string());
            context.log_message(
                &format!(
                    "Threshold {threshold} governs the positive class `{positive_name}` ({positive}); a higher threshold predicts it less often"
                ),
                LogLevel::Info,
            );
            let fitted = fitted.set_threshold(threshold);
            let predictions: Array1<usize> = fitted.predict(ds.records());
            (
                MLModel::LogisticRegression(ModelWithMeta {
                    model: fitted,
                    classes,
                }),
                predictions,
            )
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        // linfa discards the solver's `OptimizationResult`, so the achieved gradient norm is not
        // observable. Training accuracy against the majority-class baseline is the proxy signal
        // for "the solver stopped before it learned anything".
        let correct = predictions
            .iter()
            .zip(ds.targets().iter())
            .filter(|(predicted, actual)| predicted == actual)
            .count();
        let accuracy = correct as f64 / n_samples as f64;
        let baseline = counts
            .iter()
            .map(|(_, count)| *count)
            .max()
            .unwrap_or(n_samples) as f64
            / n_samples as f64;
        if accuracy <= baseline {
            context.log_message(
                &format!(
                    "Training accuracy {accuracy:.4} did not beat the majority-class baseline {baseline:.4} after at most {max_iterations} iterations (gradient tolerance {gradient_tolerance}). Raise Max Iterations, lower Alpha, or scale the features."
                ),
                LogLevel::Warn,
            );
        } else {
            context.log_message(
                &format!("Training accuracy: {accuracy:.4} (baseline {baseline:.4})"),
                LogLevel::Debug,
            );
        }

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
