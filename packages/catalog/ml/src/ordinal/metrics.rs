//! Node for **Ordinal Evaluation Metrics**
//!
//! Accuracy is the wrong yardstick for an ordered target. It scores "predicted high when the truth
//! was medium" exactly as harshly as "predicted low", so a model that is consistently one level off
//! looks identical to one that guesses at random. Every metric here is distance aware, and they
//! answer three different questions that a single headline number hides:
//!
//! - *How far off is it?* The two weighted kappas (quadratic forgives near misses, linear charges
//!   the same for every step) plus the mean absolute rank error and its macro-averaged twin, which
//!   gives each level one vote instead of letting the majority level speak for the model.
//! - *How often is it right?* The two accuracies, separating exact hits from "off by at most one".
//! - *Does it order the rows correctly?* Kendall's tau-b and the Spearman rank correlation, which
//!   ignore calibration entirely — a model shifted one level up still ranks perfectly.
//!
//! Both columns are ranked through a single level vocabulary. Ranking them independently would let
//! the same label land on a different rank in each column, which turns every distance into noise
//! without any visible failure.

use crate::ml::OrdinalOrdering;
#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, values_to_array1_ordinal};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_ordinal::{
    accuracy_within, kendall_tau_b, linear_weighted_kappa, macro_mean_absolute_error,
    mean_absolute_rank_error, quadratic_weighted_kappa, spearman_rank_correlation,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Distance-aware evaluation of an ordinal prediction column
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalMetricsResult {
    /// Quadratic weighted kappa: agreement corrected for chance, with the penalty growing with the
    /// square of the level distance. 1.0 is perfect, 0.0 is chance, negative is worse than chance.
    pub quadratic_weighted_kappa: f64,
    /// Linear weighted kappa: the same chance-corrected agreement with every level of distance
    /// costing the same. Lower than the quadratic figure whenever the misses are near misses.
    pub linear_weighted_kappa: f64,
    /// Average distance between the predicted and the true level, measured in levels
    pub mean_absolute_rank_error: f64,
    /// Mean absolute rank error computed per true level and averaged with one vote per level, so
    /// the majority level cannot hide what the model does on the rare ones.
    pub macro_mean_absolute_error: f64,
    /// Share of predictions hitting the exact level (plain accuracy, for reference only)
    pub accuracy_exact: f64,
    /// Share of predictions landing on the true level or one of its neighbours
    pub accuracy_within_one: f64,
    /// Kendall's tau-b: tie-corrected rank association. +1.0 orders the rows exactly as the truth
    /// does, 0.0 no association, -1.0 exactly backwards. Ignores calibration.
    pub kendall_tau_b: f64,
    /// Spearman rank correlation on midranks: the same ordering question as tau-b on a different
    /// scale, and less conservative under heavy ties.
    pub spearman_rank_correlation: f64,
    /// Number of rows evaluated
    pub n_samples: usize,
    /// Number of distinct levels both columns were ranked against
    pub n_levels: usize,
    /// The level labels from lowest to highest, in the rank order the metrics used
    pub levels: Vec<String>,
    /// Whether that order came from the Class Order pin or from parsing the levels as numbers
    pub ordering: OrdinalOrdering,
}

#[crate::register_node]
#[derive(Default)]
pub struct OrdinalMetricsNode {}

impl OrdinalMetricsNode {
    pub fn new() -> Self {
        OrdinalMetricsNode {}
    }
}

#[async_trait]
impl NodeLogic for OrdinalMetricsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_ordinal_metrics",
            "Ordinal Metrics",
            "Evaluate predictions for an ordered target with distance-aware metrics. Plain accuracy is inadequate here: it treats \"predicted high when the truth was medium\" exactly as harshly as \"predicted low\", so a model that is reliably one level off scores like one that guesses. Quadratic weighted kappa is the standard headline metric because it weights every miss by how far off it was and corrects for chance agreement, but it answers only one of three questions: the linear kappa and the macro-averaged error say how far off the model is under a different cost structure and on the rare levels, while Kendall's tau-b and the Spearman correlation say whether it orders the rows correctly at all.",
            "AI/ML/Ordinal",
        );
        node.set_version(2);
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
            "Execution trigger that begins the ordinal evaluation",
            VariableType::Execution,
        );

        node.add_input_pin(
            "database",
            "Database",
            "Database connection containing the predicted levels and the true levels",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "predictions_col",
            "Predictions Column",
            "Column holding the predicted level of each row. The labels must be the same ones the actuals column uses, since both columns are ranked against one shared level order.",
            VariableType::String,
        )
        .set_default_value(Some(json!("prediction")));

        node.add_input_pin(
            "actuals_col",
            "Actuals Column",
            "Column holding the true level of each row. When no Class Order is given, the level order is inferred from this column, and a predicted level that never occurs here is an error rather than a silent extra rank.",
            VariableType::String,
        )
        .set_default_value(Some(json!("target")));

        node.add_input_pin(
            "class_order",
            "Class Order",
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order (sorting them alphabetically would rank high < low < medium), so they have to be listed here.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the ordinal evaluation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "quadratic_weighted_kappa",
            "Quadratic Weighted Kappa",
            "Headline ordinal metric: chance-corrected agreement weighted by the squared level distance. 1.0 perfect, 0.0 chance, negative worse than chance.",
            VariableType::Float,
        );

        node.add_output_pin(
            "linear_weighted_kappa",
            "Linear Weighted Kappa",
            "The same chance-corrected agreement with every level of distance costing the same. Read this one instead of the quadratic kappa when a level is a level — grading scales, severity tiers, anything where two steps off is exactly twice as bad as one. Quadratic weighting charges a near miss only a quarter of a two-level miss, so it flatters a model that merely hovers next to the truth; where that discount is not real, this is the honest number and it will be the lower of the two.",
            VariableType::Float,
        );

        node.add_output_pin(
            "mean_absolute_rank_error",
            "Mean Absolute Rank Error",
            "Average miss in levels. 0.0 is perfect, 1.0 means being off by one level on average.",
            VariableType::Float,
        );

        node.add_output_pin(
            "macro_mean_absolute_error",
            "Macro Mean Absolute Error",
            "The mean absolute rank error computed per true level and averaged with one vote per level. Look here whenever the levels are imbalanced: the plain error averages over rows, so the majority level speaks for the model and a predictor that collapses onto it still scores well while missing every rare level. This metric gives the rare levels equal weight, so it is the one that moves when that happens. Levels absent from the actuals are skipped rather than counted as perfect.",
            VariableType::Float,
        );

        node.add_output_pin(
            "accuracy_exact",
            "Exact Accuracy",
            "Share of predictions hitting the exact level. Reported for reference; it ignores how far the misses are off.",
            VariableType::Float,
        );

        node.add_output_pin(
            "accuracy_within_one",
            "Accuracy Within One",
            "Share of predictions landing on the true level or one of its direct neighbours",
            VariableType::Float,
        );

        node.add_output_pin(
            "kendall_tau_b",
            "Kendall Tau-b",
            "Tie-corrected rank association: +1.0 orders the rows exactly as the truth does, 0.0 no association, -1.0 exactly backwards. This answers \"does the model rank the rows correctly\", which is a different question from \"does it land on the right level\" — a model whose every prediction is one level too high ranks perfectly and scores 1.0 here while the kappas drop. Consult it when the output feeds a sort, a triage queue or a threshold you can recalibrate, and read it against kappa to tell a miscalibrated model from a model that has learned nothing.",
            VariableType::Float,
        );

        node.add_output_pin(
            "spearman_rank_correlation",
            "Spearman Rank Correlation",
            "The same ordering question as tau-b, computed as a correlation on midranks. It is the less conservative of the two under the heavy ties ordinal data always has, so it reads higher than tau-b on the same predictions; prefer tau-b when you need a defensible figure and this one when comparing against Spearman values reported elsewhere. Like tau-b it ignores calibration entirely.",
            VariableType::Float,
        );

        node.add_output_pin(
            "n_samples",
            "Samples",
            "Number of rows evaluated",
            VariableType::Integer,
        );

        node.add_output_pin(
            "n_levels",
            "Levels",
            "Number of distinct levels both columns were ranked against",
            VariableType::Integer,
        );

        node.add_output_pin(
            "result",
            "Result",
            "All ordinal metrics plus the resolved level order they were computed against",
            VariableType::Struct,
        )
        .set_schema::<OrdinalMetricsResult>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let predictions_col: String = context.evaluate_pin("predictions_col").await?;
        let actuals_col: String = context.evaluate_pin("actuals_col").await?;
        let class_order: String = context
            .evaluate_pin("class_order")
            .await
            .unwrap_or_default();

        let explicit_levels: Vec<String> = class_order
            .split(',')
            .map(|level| level.trim().to_string())
            .filter(|level| !level.is_empty())
            .collect();

        let records = {
            let cached_db = database.load(context).await?;
            cached_db.ensure_flushed().await?;
            let database = cached_db.db.read().await;
            let schema = database.schema().await?;
            let existing_cols: HashSet<String> =
                schema.fields.iter().map(|f| f.name().clone()).collect();

            if !existing_cols.contains(&predictions_col) {
                return Err(anyhow!(
                    "Database doesn't contain predictions column `{}`!",
                    predictions_col
                ));
            }
            if !existing_cols.contains(&actuals_col) {
                return Err(anyhow!(
                    "Database doesn't contain actuals column `{}`!",
                    actuals_col
                ));
            }

            // Comparing a column against itself is a legitimate sanity check, so ask for each
            // column once instead of projecting a duplicate.
            let mut projection = vec![predictions_col.clone()];
            if actuals_col != predictions_col {
                projection.push(actuals_col.clone());
            }

            database
                .filter("true", Some(projection), MAX_ML_PREDICTION_RECORDS, 0)
                .await?
        };

        if records.is_empty() {
            return Err(anyhow!("No records found in database"));
        }
        if records.len() >= MAX_ML_PREDICTION_RECORDS {
            context.log_message(
                &format!(
                    "Evaluation is capped at {MAX_ML_PREDICTION_RECORDS} rows; the metrics describe that sample, not the full table"
                ),
                LogLevel::Warn,
            );
        }

        // One vocabulary for both columns: the explicit order when given, otherwise the order
        // inferred from the actuals. Ranking the columns independently would let the same label
        // take a different rank in each of them, and every distance below would be nonsense.
        let (actual_ranks, _classes, levels) = values_to_array1_ordinal(
            &records,
            &actuals_col,
            (!explicit_levels.is_empty()).then_some(explicit_levels.as_slice()),
        )?;

        let order_source = match levels.ordering {
            OrdinalOrdering::Explicit => "taken from the Class Order pin".to_string(),
            OrdinalOrdering::Numeric => {
                format!("inferred from the numeric levels of `{actuals_col}`")
            }
        };

        let (predicted_ranks, _, _) =
            values_to_array1_ordinal(&records, &predictions_col, Some(levels.labels.as_slice()))
                .map_err(|err| {
                    anyhow!(
                        "Could not rank the predicted levels in `{predictions_col}` against the level order [{}] ({order_source}): {err}. Both columns have to speak the same level labels, otherwise the same label would take a different rank in each column and every distance-based metric would be meaningless.",
                        levels.labels.join(", ")
                    )
                })?;

        let predicted = predicted_ranks.to_vec();
        let actual = actual_ranks.to_vec();
        let n_samples = actual.len();
        let n_levels = levels.labels.len();

        if predicted.len() != n_samples {
            return Err(anyhow!(
                "Ranked {} predicted levels but {n_samples} actual levels; every row needs both a prediction and a truth",
                predicted.len()
            ));
        }
        if n_samples == 0 {
            return Err(anyhow!(
                "No rows left to evaluate after ranking `{predictions_col}` and `{actuals_col}`"
            ));
        }
        if n_levels < 2 {
            return Err(anyhow!(
                "Ordinal metrics need at least 2 ordered levels, found {n_levels} in `{actuals_col}` [{}]. A single level makes every distance zero and kappa undefined.",
                levels.labels.join(", ")
            ));
        }

        let quadratic_weighted_kappa = quadratic_weighted_kappa(&predicted, &actual, n_levels)
            .map_err(|err| {
                anyhow!("Quadratic weighted kappa over {n_levels} levels failed: {err}")
            })?;
        let linear_weighted_kappa = linear_weighted_kappa(&predicted, &actual, n_levels)
            .map_err(|err| anyhow!("Linear weighted kappa over {n_levels} levels failed: {err}"))?;
        let mean_absolute_rank_error = mean_absolute_rank_error(&predicted, &actual)
            .map_err(|err| anyhow!("Mean absolute rank error failed: {err}"))?;
        let macro_mean_absolute_error = macro_mean_absolute_error(&predicted, &actual, n_levels)
            .map_err(|err| {
                anyhow!("Macro mean absolute error over {n_levels} levels failed: {err}")
            })?;
        let accuracy_exact = accuracy_within(&predicted, &actual, 0)
            .map_err(|err| anyhow!("Exact accuracy failed: {err}"))?;
        let accuracy_within_one = accuracy_within(&predicted, &actual, 1)
            .map_err(|err| anyhow!("Accuracy within one level failed: {err}"))?;
        let kendall_tau_b = kendall_tau_b(&predicted, &actual, n_levels)
            .map_err(|err| anyhow!("Kendall's tau-b over {n_levels} levels failed: {err}"))?;
        let spearman_rank_correlation = spearman_rank_correlation(&predicted, &actual, n_levels)
            .map_err(|err| {
                anyhow!("Spearman rank correlation over {n_levels} levels failed: {err}")
            })?;

        context.log_message(
            &format!(
                "Ordinal metrics over {n_samples} rows and {n_levels} levels [{}] ({order_source}): QWK {:.4}, LWK {:.4}, MARE {:.4}, MMAE {:.4}, exact {:.4}, within one {:.4}, tau-b {:.4}, Spearman {:.4}",
                levels.labels.join(" < "),
                quadratic_weighted_kappa,
                linear_weighted_kappa,
                mean_absolute_rank_error,
                macro_mean_absolute_error,
                accuracy_exact,
                accuracy_within_one,
                kendall_tau_b,
                spearman_rank_correlation
            ),
            LogLevel::Debug,
        );
        if quadratic_weighted_kappa <= 0.0 {
            context.log_message(
                &format!(
                    "Quadratic weighted kappa is {quadratic_weighted_kappa:.4}, at or below what chance would achieve. Check that the level order is right and that the model is not collapsing onto a single level."
                ),
                LogLevel::Warn,
            );
        }
        if macro_mean_absolute_error > 0.0
            && macro_mean_absolute_error >= mean_absolute_rank_error * 2.0
        {
            context.log_message(
                &format!(
                    "Macro mean absolute error {macro_mean_absolute_error:.4} is far above the plain {mean_absolute_rank_error:.4}, so the error is concentrated in the rarer levels while the majority level carries the headline figure. Judge this model by the macro figure."
                ),
                LogLevel::Warn,
            );
        }
        if kendall_tau_b >= 0.7 && quadratic_weighted_kappa <= 0.3 {
            context.log_message(
                &format!(
                    "Kendall's tau-b is {kendall_tau_b:.4} but quadratic weighted kappa only {quadratic_weighted_kappa:.4}: the model orders the rows well and lands on the wrong level, which is a calibration problem (shifted thresholds), not a lack of signal."
                ),
                LogLevel::Warn,
            );
        }

        let result = OrdinalMetricsResult {
            quadratic_weighted_kappa,
            linear_weighted_kappa,
            mean_absolute_rank_error,
            macro_mean_absolute_error,
            accuracy_exact,
            accuracy_within_one,
            kendall_tau_b,
            spearman_rank_correlation,
            n_samples,
            n_levels,
            levels: levels.labels,
            ordering: levels.ordering,
        };

        context
            .set_pin_value(
                "quadratic_weighted_kappa",
                json!(result.quadratic_weighted_kappa),
            )
            .await?;
        context
            .set_pin_value("linear_weighted_kappa", json!(result.linear_weighted_kappa))
            .await?;
        context
            .set_pin_value(
                "mean_absolute_rank_error",
                json!(result.mean_absolute_rank_error),
            )
            .await?;
        context
            .set_pin_value(
                "macro_mean_absolute_error",
                json!(result.macro_mean_absolute_error),
            )
            .await?;
        context
            .set_pin_value("accuracy_exact", json!(result.accuracy_exact))
            .await?;
        context
            .set_pin_value("accuracy_within_one", json!(result.accuracy_within_one))
            .await?;
        context
            .set_pin_value("kendall_tau_b", json!(result.kendall_tau_b))
            .await?;
        context
            .set_pin_value(
                "spearman_rank_correlation",
                json!(result.spearman_rank_correlation),
            )
            .await?;
        context
            .set_pin_value("n_samples", json!(result.n_samples as i64))
            .await?;
        context
            .set_pin_value("n_levels", json!(result.n_levels as i64))
            .await?;
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
