//! Node for **ROC-AUC, Log Loss and the ROC curve**
//!
//! Threshold-free evaluation of a binary classifier. Accuracy and the confusion matrix judge a
//! single decision threshold; ROC-AUC ranks every threshold at once and log loss penalises
//! over-confident mistakes. Both need calibrated probabilities rather than hard labels, which is
//! exactly what Logistic Regression (and the `confidence` field of a prediction) produces — this
//! node is the payoff for training a model that reports probabilities instead of bare classes.

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
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use flow_like_types::{Value, anyhow};
#[cfg(feature = "execute")]
use linfa::metrics::BinaryClassification;
#[cfg(feature = "execute")]
use linfa::prelude::Pr;
#[cfg(feature = "execute")]
use ndarray::Array1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// A single point of the Receiver-Operating-Characteristic curve
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RocPoint {
    /// Probability threshold this point was measured at. `null` for the closing (0, 0) endpoint,
    /// which corresponds to classifying everything as negative.
    pub threshold: Option<f64>,
    /// False positive rate at this threshold, the x axis of the curve
    pub false_positive_rate: f64,
    /// True positive rate (recall) at this threshold, the y axis of the curve
    pub true_positive_rate: f64,
}

/// ROC-AUC, log loss and the full ROC curve of a binary classifier
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RocAucResult {
    /// Area under the ROC curve. 0.5 is a coin flip, 1.0 is a perfect ranking.
    pub auc: f64,
    /// Mean binary cross-entropy of the predicted probabilities. Lower is better.
    pub log_loss: f64,
    /// Curve points ordered by ascending false positive rate, ready to be charted
    pub curve: Vec<RocPoint>,
    /// Number of samples evaluated
    pub n_samples: usize,
    /// Number of samples whose true label is positive
    pub n_positive: usize,
    /// Number of samples whose true label is negative
    pub n_negative: usize,
}

#[crate::register_node]
#[derive(Default)]
pub struct RocAucNode {}

impl RocAucNode {
    pub fn new() -> Self {
        RocAucNode {}
    }
}

#[async_trait]
impl NodeLogic for RocAucNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_roc_auc",
            "ROC-AUC & Log Loss",
            "Threshold-free evaluation of a binary classifier: area under the ROC curve, log loss and the curve points. This is the payoff for Logistic Regression producing calibrated probabilities instead of bare class labels.",
            "AI/ML/Metrics",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins the ROC evaluation",
            VariableType::Execution,
        );

        node.add_input_pin(
            "database",
            "Database",
            "Database connection containing the predicted probabilities and the true labels",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "probabilities_col",
            "Probabilities Column",
            "Column holding P(positive class) for each row, between 0 and 1 — the probability of the class named in Positive Label, NOT the probability of whichever class was predicted. No node writes this column for you: Predict in Database mode writes the predicted class only, and `confidence` is a field on the struct its Vector mode returns for one row, so build the column by looping rows through Vector mode. Convert as you go, because `confidence` is the winning class's probability: use it directly where the prediction is the positive class, and 1 - confidence elsewhere. A raw decision value or an uncalibrated score produces a meaningless curve.",
            VariableType::String,
        )
        .set_default_value(Some(json!("probability")));

        node.add_input_pin(
            "actuals_col",
            "Actuals Column",
            "Column holding the true binary label of each sample",
            VariableType::String,
        )
        .set_default_value(Some(json!("target")));

        node.add_input_pin(
            "positive_label",
            "Positive Label",
            "Value of the actuals column that counts as the positive class. Strings are compared literally, numbers numerically; booleans are always taken as-is.",
            VariableType::String,
        )
        .set_default_value(Some(json!("1")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the ROC evaluation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "auc",
            "AUC",
            "Area under the ROC curve (0.5 = random, 1.0 = perfect)",
            VariableType::Float,
        );

        node.add_output_pin(
            "log_loss",
            "Log Loss",
            "Mean binary cross-entropy of the predicted probabilities (lower is better)",
            VariableType::Float,
        );

        node.add_output_pin(
            "result",
            "Result",
            "AUC, log loss and the ROC curve points ordered by ascending false positive rate",
            VariableType::Struct,
        )
        .set_schema::<RocAucResult>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use crate::ml::MAX_ML_PREDICTION_RECORDS;
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let probabilities_col: String = context.evaluate_pin("probabilities_col").await?;
        let actuals_col: String = context.evaluate_pin("actuals_col").await?;
        let positive_label: String = context.evaluate_pin("positive_label").await?;

        let records = {
            let cached_db = database.load(context).await?;
            cached_db.ensure_flushed().await?;
            let database = cached_db.db.read().await;
            let schema = database.schema().await?;
            let existing_cols: HashSet<String> =
                schema.fields.iter().map(|f| f.name().clone()).collect();

            if !existing_cols.contains(&probabilities_col) {
                return Err(anyhow!(
                    "Database doesn't contain probabilities column `{}`!",
                    probabilities_col
                ));
            }
            if !existing_cols.contains(&actuals_col) {
                return Err(anyhow!(
                    "Database doesn't contain actuals column `{}`!",
                    actuals_col
                ));
            }

            database
                .filter(
                    "true",
                    Some(vec![probabilities_col.clone(), actuals_col.clone()]),
                    MAX_ML_PREDICTION_RECORDS,
                    0,
                )
                .await?
        };

        if records.is_empty() {
            return Err(anyhow!("No records found in database"));
        }

        let mut probabilities = Vec::with_capacity(records.len());
        let mut truths: Vec<bool> = Vec::with_capacity(records.len());
        let mut roc_scores: Vec<Pr> = Vec::with_capacity(records.len());
        let mut min_probability = f64::INFINITY;
        let mut max_probability = f64::NEG_INFINITY;

        for (row, record) in records.iter().enumerate() {
            let raw = record
                .get(&probabilities_col)
                .and_then(|value| value.as_f64())
                .ok_or_else(|| {
                    anyhow!("Row {row}: column `{probabilities_col}` is not a valid float")
                })?;

            if !raw.is_finite() || !(0.0..=1.0).contains(&raw) {
                return Err(anyhow!(
                    "Row {row}: value {raw} in `{probabilities_col}` is outside [0, 1]. ROC-AUC and log loss need calibrated probabilities, not raw decision scores."
                ));
            }

            min_probability = min_probability.min(raw);
            max_probability = max_probability.max(raw);
            // linfa keeps probabilities as f32 and `Pr::new` asserts the [0, 1] range, so clamp
            // after the narrowing cast: a rounding artefact must not become a panic.
            probabilities.push(Pr::new((raw as f32).clamp(0.0, 1.0)));
            roc_scores.push(Pr::new((roc_rescale(raw) as f32).clamp(0.0, 1.0)));

            let label = record
                .get(&actuals_col)
                .ok_or_else(|| anyhow!("Row {row}: missing actuals column `{actuals_col}`"))?;
            truths.push(label_to_bool(label, &positive_label, row, &actuals_col)?);
        }

        let n_samples = truths.len();
        let n_positive = truths.iter().filter(|label| **label).count();
        let n_negative = n_samples - n_positive;

        if n_positive == 0 || n_negative == 0 {
            return Err(anyhow!(
                "ROC-AUC needs both classes present, but `{actuals_col}` yields {n_positive} positive and {n_negative} negative samples for positive label `{positive_label}`. linfa normalises the curve by the per-class counts, so a one-sided column produces NaN."
            ));
        }

        if max_probability - min_probability <= f64::EPSILON {
            return Err(anyhow!(
                "All predicted probabilities in `{probabilities_col}` are {max_probability}. A constant score carries no ranking, and the trapezoidal integration would report an area of 0 rather than the true 0.5."
            ));
        }

        // Log loss is a calibration metric and must see the probabilities the user supplied; the
        // ROC is rank-based, so it sees the rescaled copy that avoids linfa's dead zone.
        let probabilities = Array1::from(probabilities);
        let roc_scores = Array1::from(roc_scores);
        let roc = roc_scores.roc(&truths)?;
        let log_loss = f64::from(probabilities.log_loss(&truths)?);
        let auc = f64::from(roc.area_under_curve());

        if !auc.is_finite() || !log_loss.is_finite() {
            return Err(anyhow!(
                "ROC evaluation produced non-finite values (auc={auc}, log_loss={log_loss})"
            ));
        }

        // `get_curve` walks the samples from the lowest score upwards and reports
        // (share of positives below the threshold, share of negatives below it), i.e.
        // (1 - TPR, 1 - FPR). Flip both axes and traverse backwards to obtain the conventional
        // ROC ordering with an ascending false positive rate. The last raw point closes the curve
        // and has no threshold of its own, hence the `get`.
        let raw_curve = roc.get_curve();
        let thresholds = roc.get_thresholds();
        let mut curve = Vec::with_capacity(raw_curve.len());
        for (index, (positives_below, negatives_below)) in raw_curve.iter().enumerate().rev() {
            curve.push(RocPoint {
                threshold: thresholds
                    .get(index)
                    .map(|value| roc_unscale(f64::from(*value))),
                false_positive_rate: 1.0 - f64::from(*negatives_below),
                true_positive_rate: 1.0 - f64::from(*positives_below),
            });
        }

        context.log_message(
            &format!(
                "ROC-AUC: {auc:.4}, Log Loss: {log_loss:.4} over {n_samples} samples ({n_positive} positive / {n_negative} negative), {} curve points",
                curve.len()
            ),
            LogLevel::Debug,
        );

        let result = RocAucResult {
            auc,
            log_loss,
            curve,
            n_samples,
            n_positive,
            n_negative,
        };

        context.set_pin_value("auc", json!(auc)).await?;
        context.set_pin_value("log_loss", json!(log_loss)).await?;
        context.set_pin_value("result", json!(result)).await?;
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

/// Lower bound linfa's `roc()` requires a score to exceed before it emits the curve's (0, 0)
/// anchor. Anything at or below it makes the curve start partway up and understates the area.
#[cfg(feature = "execute")]
const ROC_DEAD_ZONE: f64 = 1e-9;

/// Maps `[0, 1]` onto `[ROC_DEAD_ZONE, 1 - ROC_DEAD_ZONE]`.
///
/// Strictly increasing, so every pairwise ranking — and therefore the AUC — is unchanged, but no
/// score lands in the range where linfa drops the anchor point. Applied only to the ROC input;
/// log loss keeps the probabilities as given.
#[cfg(feature = "execute")]
fn roc_rescale(probability: f64) -> f64 {
    ROC_DEAD_ZONE + probability * (1.0 - 2.0 * ROC_DEAD_ZONE)
}

/// Inverse of [`roc_rescale`], so exported thresholds are on the user's original scale.
#[cfg(feature = "execute")]
fn roc_unscale(threshold: f64) -> f64 {
    ((threshold - ROC_DEAD_ZONE) / (1.0 - 2.0 * ROC_DEAD_ZONE)).clamp(0.0, 1.0)
}

#[cfg(feature = "execute")]
fn label_to_bool(value: &Value, positive_label: &str, row: usize, column: &str) -> Result<bool> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::Number(number) => {
            let numeric = number.as_f64().ok_or_else(|| {
                anyhow!("Row {row}: column `{column}` holds a number that is not representable")
            })?;
            match positive_label.trim().parse::<f64>() {
                // The label survived a JSON round trip, so compare with a tolerance instead of `==`.
                Ok(target) => Ok((numeric - target).abs() <= f64::EPSILON),
                // Falling back to "any non-zero counts as positive" would silently score against
                // a class the user never named.
                Err(_) => Err(anyhow!(
                    "Row {row}: column `{column}` holds numbers but Positive Label `{positive_label}` is not a number. Set Positive Label to the numeric class id that counts as positive."
                )),
            }
        }
        Value::String(text) => Ok(text == positive_label),
        other => Err(anyhow!(
            "Row {row}: column `{column}` holds `{other}`, expected a boolean, a number or a string label"
        )),
    }
}
