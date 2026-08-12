//! AutoML for **ordered** targets.
//!
//! The nominal Auto Classifier cannot stand in for this. It resolves the target with
//! `values_to_array1_target`, which throws the level order away, and ranks by accuracy or macro-F1,
//! which score "predicted the lowest level when the truth was the highest" exactly as harshly as a
//! one-level miss. A leaderboard mixing ordinal with nominal candidates would also compare models
//! that are not comparable. So this node keeps the ordinal contract end to end: the target is
//! resolved into ranks with a declared level order, every family is fitted against the same
//! declared level count, and the ranking metric is distance-aware.

#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, OrdinalOrdering, values_to_array1_ordinal,
    values_to_array2_f64,
};
use crate::ml::{NodeMLModel, OrdinalLevels};
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
use flow_like_ordinal::{
    Activation, AdjacentCategory, ContinuationRatio, Link, Margin, OrdinalHead, OrdinalLogistic,
    OrdinalLoss, OrdinalNeural, OrdinalNeuralParams, OrdinalRidge, kendall_tau_b,
    linear_weighted_kappa, macro_mean_absolute_error, mean_absolute_rank_error,
    quadratic_weighted_kappa, spearman_rank_correlation,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
#[cfg(feature = "execute")]
use flow_like_types::rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::{Fit, Predict};
#[cfg(feature = "execute")]
use ndarray::{Array1, Array2, Axis};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(feature = "execute")]
use std::collections::HashSet;
#[cfg(feature = "execute")]
use std::time::Instant;

/// One configuration that survived cross-validation, as it appears on the leaderboard.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalAutoMLEntry {
    /// Stable model kind, the same identifier the rest of the catalog uses for this model:
    /// `OrdinalLogistic`, `OrdinalRidge`, `OrdinalContinuationRatio`, `OrdinalAdjacentCategory` or
    /// `OrdinalNeural`.
    pub model_type: String,
    /// The configuration in words, e.g. `Support Vector Ordinal Regression (all-threshold loss,
    /// hinge margin)`. Several variants can share one model type.
    pub variant: String,
    /// Hyperparameters this entry was fitted with. Only values the node sets explicitly appear;
    /// anything absent was left at the estimator's own default.
    pub params: HashMap<String, Value>,
    /// Mean score over the folds, in the units of the chosen metric.
    pub cv_score: f64,
    /// Seconds spent fitting and scoring this configuration across all folds.
    pub train_time_secs: f64,
    /// Position in the leaderboard, 1 being the best under the chosen metric *and its direction*.
    pub rank: usize,
}

/// A configuration that was dropped from the leaderboard because it could not be fitted.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalAutoMLSkip {
    /// The configuration in words, matching the leaderboard's `variant` naming.
    pub variant: String,
    /// Which fold it failed on and what the estimator reported.
    pub reason: String,
}

/// Complete ordinal AutoML results.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalAutoMLResult {
    /// Every configuration that finished, best first.
    pub leaderboard: Vec<OrdinalAutoMLEntry>,
    /// Configurations excluded because a fold could not be fitted, with the reason.
    pub skipped: Vec<OrdinalAutoMLSkip>,
    /// Index of the winning entry inside `leaderboard`.
    pub best_index: usize,
    /// Number of configurations on the leaderboard. Excluded ones are counted in `skipped`.
    pub total_models_tried: usize,
    /// Wall clock seconds for the whole sweep, including data loading and the final refit.
    pub total_time_secs: f64,
    /// Metric the leaderboard was ranked by.
    pub metric: String,
    /// False for the two error metrics, where the SMALLEST `cv_score` is the winner.
    pub higher_is_better: bool,
    /// Number of ordered levels every candidate was fitted against.
    pub n_levels: usize,
    /// Number of cross-validation folds.
    pub n_folds: usize,
    /// Rows the sweep ran on.
    pub n_samples: usize,
}

/// L2 penalties swept for the Ordinal Ridge family.
#[cfg(feature = "execute")]
const RIDGE_ALPHAS: [f64; 3] = [0.1, 1.0, 10.0];

/// Backbone shared by both neural candidates: a single small hidden layer.
///
/// Deliberately modest, and deliberately identical for both heads. This is a SCREENING sweep whose
/// question is "does a non-linear boundary exist at all", which one hidden layer already answers,
/// and the two heads are only comparable to each other if they sit on the same backbone. Widening
/// or deepening it here would multiply the cost of the one family that already dominates the
/// runtime while telling the user nothing new - the Ordinal Grid Search node is where the winning
/// family gets its hyperparameters tuned.
#[cfg(feature = "execute")]
const NEURAL_HIDDEN_LAYERS: [usize; 1] = [16];

/// Non-linearity between the neural backbone's layers, held constant across both heads for the same
/// reason as the layer widths.
#[cfg(feature = "execute")]
const NEURAL_ACTIVATION: Activation = Activation::Relu;

/// Seed for the neural weight initialization.
///
/// Fixed on purpose. The neural objective is the only non-convex one in the sweep, so an unseeded
/// initialization would move the network's score between runs and let it trade places with a linear
/// family it never actually beat - the same reason the fold shuffle is seeded rather than left to
/// chance. The winner's refit reuses this seed, so the model handed out is the one that was scored.
///
/// Kept well below 2^53 because it is reported on the leaderboard: a larger value survives Rust and
/// JSON intact but loses its low digits the moment a viewer reads it as a double, leaving a printed
/// seed that no longer reproduces the fit.
#[cfg(feature = "execute")]
const NEURAL_SEED: u64 = 0x5EED_0A17;

/// The one neural configuration, built in a single place so cross-validation and the winner's refit
/// cannot diverge: a leaderboard entry earned under one architecture and rebuilt under another would
/// be a silent lie about what the user is holding.
#[cfg(feature = "execute")]
fn neural_params(head: OrdinalHead, n_levels: usize) -> OrdinalNeuralParams<f64> {
    OrdinalNeural::<f64>::params()
        .head(head)
        .hidden_layers(&NEURAL_HIDDEN_LAYERS)
        .activation(NEURAL_ACTIVATION)
        .seed(NEURAL_SEED)
        .n_levels(n_levels)
}

/// The backbone written the way the leaderboard shows it, e.g. `16`.
#[cfg(feature = "execute")]
fn neural_hidden_layer_summary() -> String {
    NEURAL_HIDDEN_LAYERS
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Leaderboard `model_type` values, mirroring `MLModel::kind()` in `crate::ml`. The winner's own
/// value is read back off `kind()` after the refit, so a drift here can never reach the output pin.
#[cfg(feature = "execute")]
const KIND_LOGISTIC: &str = "OrdinalLogistic";
#[cfg(feature = "execute")]
const KIND_RIDGE: &str = "OrdinalRidge";
#[cfg(feature = "execute")]
const KIND_CONTINUATION_RATIO: &str = "OrdinalContinuationRatio";
#[cfg(feature = "execute")]
const KIND_ADJACENT_CATEGORY: &str = "OrdinalAdjacentCategory";
#[cfg(feature = "execute")]
const KIND_NEURAL: &str = "OrdinalNeural";

const METRIC_QUADRATIC_KAPPA: &str = "Quadratic Kappa";
const METRIC_LINEAR_KAPPA: &str = "Linear Kappa";
const METRIC_MEAN_ABSOLUTE_RANK_ERROR: &str = "Mean Rank Error";
const METRIC_MACRO_MEAN_ABSOLUTE_ERROR: &str = "Macro Rank Error";
const METRIC_KENDALL_TAU_B: &str = "Kendall Tau-b";
const METRIC_SPEARMAN: &str = "Spearman";

const METRICS: [&str; 6] = [
    METRIC_QUADRATIC_KAPPA,
    METRIC_LINEAR_KAPPA,
    METRIC_MEAN_ABSOLUTE_RANK_ERROR,
    METRIC_MACRO_MEAN_ABSOLUTE_ERROR,
    METRIC_KENDALL_TAU_B,
    METRIC_SPEARMAN,
];

/// The metric the leaderboard is ranked by.
#[cfg(feature = "execute")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrdinalMetric {
    QuadraticWeightedKappa,
    LinearWeightedKappa,
    MeanAbsoluteRankError,
    MacroMeanAbsoluteError,
    KendallTauB,
    SpearmanRankCorrelation,
}

#[cfg(feature = "execute")]
impl OrdinalMetric {
    fn parse(value: &str) -> Result<Self> {
        match value {
            METRIC_QUADRATIC_KAPPA => Ok(OrdinalMetric::QuadraticWeightedKappa),
            METRIC_LINEAR_KAPPA => Ok(OrdinalMetric::LinearWeightedKappa),
            METRIC_MEAN_ABSOLUTE_RANK_ERROR => Ok(OrdinalMetric::MeanAbsoluteRankError),
            METRIC_MACRO_MEAN_ABSOLUTE_ERROR => Ok(OrdinalMetric::MacroMeanAbsoluteError),
            METRIC_KENDALL_TAU_B => Ok(OrdinalMetric::KendallTauB),
            METRIC_SPEARMAN => Ok(OrdinalMetric::SpearmanRankCorrelation),
            other => Err(anyhow!(
                "Unknown metric `{other}`, expected one of: {}",
                METRICS.join(", ")
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            OrdinalMetric::QuadraticWeightedKappa => METRIC_QUADRATIC_KAPPA,
            OrdinalMetric::LinearWeightedKappa => METRIC_LINEAR_KAPPA,
            OrdinalMetric::MeanAbsoluteRankError => METRIC_MEAN_ABSOLUTE_RANK_ERROR,
            OrdinalMetric::MacroMeanAbsoluteError => METRIC_MACRO_MEAN_ABSOLUTE_ERROR,
            OrdinalMetric::KendallTauB => METRIC_KENDALL_TAU_B,
            OrdinalMetric::SpearmanRankCorrelation => METRIC_SPEARMAN,
        }
    }
}

/// Scores predictions under `metric`, returning the value **together with** the direction that
/// counts as an improvement.
///
/// Four of the six metrics are agreement measures where a higher value wins, while
/// `MeanAbsoluteRankError` and `MacroMeanAbsoluteError` are error measures where a lower value
/// wins. Ranking an error metric the wrong way round crowns the *worst* model and leaves a
/// leaderboard that looks entirely plausible, so the value and its direction are produced by one
/// match and handed out as a pair: no caller can obtain a score without also being handed the
/// direction to sort it by.
#[cfg(feature = "execute")]
fn score_predictions(
    metric: OrdinalMetric,
    predicted: &[usize],
    actual: &[usize],
    n_levels: usize,
) -> Result<(f64, bool)> {
    let (value, higher_is_better) = match metric {
        OrdinalMetric::QuadraticWeightedKappa => {
            (quadratic_weighted_kappa(predicted, actual, n_levels), true)
        }
        OrdinalMetric::LinearWeightedKappa => {
            (linear_weighted_kappa(predicted, actual, n_levels), true)
        }
        OrdinalMetric::MeanAbsoluteRankError => {
            (mean_absolute_rank_error(predicted, actual), false)
        }
        OrdinalMetric::MacroMeanAbsoluteError => (
            macro_mean_absolute_error(predicted, actual, n_levels),
            false,
        ),
        OrdinalMetric::KendallTauB => (kendall_tau_b(predicted, actual, n_levels), true),
        OrdinalMetric::SpearmanRankCorrelation => {
            (spearman_rank_correlation(predicted, actual, n_levels), true)
        }
    };
    let value = value.map_err(|err| anyhow!("Scoring with `{}` failed: {err}", metric.as_str()))?;
    Ok((value, higher_is_better))
}

/// One model configuration in the sweep.
#[cfg(feature = "execute")]
#[derive(Debug, Clone, Copy)]
enum Candidate {
    /// Cumulative-link threshold model: one shared coefficient vector plus ordered cut points.
    CumulativeLink(Link),
    /// Threshold model fitted with the all-threshold loss; the hinge margin makes it SVOR.
    AllThreshold(Margin),
    /// Rank regression with learned cut points, swept over the L2 penalty.
    Ridge(f64),
    /// Sequential progression, `P(stop at k | reached k)`.
    ContinuationRatio,
    /// Contrasts between neighbouring levels.
    AdjacentCategory,
    /// Small MLP under a rank-consistent head; the only non-linear family in the sweep.
    Neural(OrdinalHead),
}

#[cfg(feature = "execute")]
impl Candidate {
    fn model_type(self) -> &'static str {
        match self {
            Candidate::CumulativeLink(_) | Candidate::AllThreshold(_) => KIND_LOGISTIC,
            Candidate::Ridge(_) => KIND_RIDGE,
            Candidate::ContinuationRatio => KIND_CONTINUATION_RATIO,
            Candidate::AdjacentCategory => KIND_ADJACENT_CATEGORY,
            Candidate::Neural(_) => KIND_NEURAL,
        }
    }

    fn variant(self) -> String {
        match self {
            Candidate::CumulativeLink(Link::Logit) => {
                "Proportional Odds (cumulative logit)".to_string()
            }
            Candidate::CumulativeLink(Link::Probit) => {
                "Ordered Probit (cumulative probit)".to_string()
            }
            Candidate::CumulativeLink(link) => format!("Cumulative Link ({link:?})"),
            Candidate::AllThreshold(Margin::Hinge) => {
                // Hinge margin + all-threshold is Chu & Keerthi's SVOR-IMC, so it is named for what
                // it is rather than as another loss setting.
                "Support Vector Ordinal Regression (all-threshold loss, hinge margin)".to_string()
            }
            Candidate::AllThreshold(margin) => {
                format!("All-Threshold ({margin:?} margin)")
            }
            Candidate::Ridge(alpha) => format!("Ordinal Ridge (alpha {alpha})"),
            Candidate::ContinuationRatio => "Continuation Ratio".to_string(),
            Candidate::AdjacentCategory => "Adjacent Category".to_string(),
            // Both heads share one model type, so the head has to be what tells them apart on the
            // leaderboard - the same way the SVOR entry is named for its loss and margin.
            Candidate::Neural(OrdinalHead::Coral) => format!(
                "Neural Ordinal (CORAL head, hidden layers [{}])",
                neural_hidden_layer_summary()
            ),
            Candidate::Neural(OrdinalHead::Corn) => format!(
                "Neural Ordinal (CORN head, hidden layers [{}])",
                neural_hidden_layer_summary()
            ),
        }
    }

    fn params(self) -> HashMap<String, Value> {
        let mut params = HashMap::new();
        match self {
            Candidate::CumulativeLink(link) => {
                params.insert("loss".to_string(), json!("CumulativeLink"));
                params.insert("link".to_string(), json!(format!("{link:?}")));
            }
            Candidate::AllThreshold(margin) => {
                params.insert("loss".to_string(), json!("AllThreshold"));
                params.insert("margin".to_string(), json!(format!("{margin:?}")));
            }
            Candidate::Ridge(alpha) => {
                params.insert("alpha".to_string(), json!(alpha));
            }
            Candidate::ContinuationRatio | Candidate::AdjacentCategory => {}
            Candidate::Neural(head) => {
                params.insert("head".to_string(), json!(format!("{head:?}")));
                params.insert(
                    "hidden_layers".to_string(),
                    json!(NEURAL_HIDDEN_LAYERS.to_vec()),
                );
                params.insert(
                    "activation".to_string(),
                    json!(format!("{NEURAL_ACTIVATION:?}")),
                );
                params.insert("seed".to_string(), json!(NEURAL_SEED));
            }
        }
        params
    }

    /// Fits on the training split and predicts the held-out rows.
    fn fit_predict(
        self,
        train: &DatasetBase<Array2<f64>, Array1<usize>>,
        validation: &Array2<f64>,
        n_levels: usize,
    ) -> Result<Array1<usize>> {
        match self {
            Candidate::CumulativeLink(link) => {
                let model = OrdinalLogistic::<f64>::params()
                    .link(link)
                    .loss(OrdinalLoss::CumulativeLink)
                    .n_levels(n_levels)
                    .fit(train)
                    .map_err(|err| anyhow!("{err}"))?;
                Ok(model.predict(validation))
            }
            Candidate::AllThreshold(margin) => {
                let model = OrdinalLogistic::<f64>::params()
                    .loss(OrdinalLoss::AllThreshold)
                    .margin(margin)
                    .n_levels(n_levels)
                    .fit(train)
                    .map_err(|err| anyhow!("{err}"))?;
                Ok(model.predict(validation))
            }
            Candidate::Ridge(alpha) => {
                let model = OrdinalRidge::<f64>::params()
                    .alpha(alpha)
                    .n_levels(n_levels)
                    .fit(train)
                    .map_err(|err| anyhow!("{err}"))?;
                Ok(model.predict(validation))
            }
            Candidate::ContinuationRatio => {
                let model = ContinuationRatio::<f64>::params()
                    .n_levels(n_levels)
                    .fit(train)
                    .map_err(|err| anyhow!("{err}"))?;
                Ok(model.predict(validation))
            }
            Candidate::AdjacentCategory => {
                let model = AdjacentCategory::<f64>::params()
                    .n_levels(n_levels)
                    .fit(train)
                    .map_err(|err| anyhow!("{err}"))?;
                Ok(model.predict(validation))
            }
            Candidate::Neural(head) => {
                let model = neural_params(head, n_levels)
                    .fit(train)
                    .map_err(|err| anyhow!("{err}"))?;
                Ok(model.predict(validation))
            }
        }
    }

    /// Refits the configuration on the full dataset and wraps it as a catalog model.
    fn fit_final(
        self,
        dataset: &DatasetBase<Array2<f64>, Array1<usize>>,
        n_levels: usize,
        classes: HashMap<usize, String>,
    ) -> Result<MLModel> {
        let classes = Some(classes);
        let describe = |err: flow_like_ordinal::OrdinalError| {
            anyhow!(
                "`{}` won cross-validation but failed to refit on the full dataset: {err}",
                self.variant()
            )
        };
        let model = match self {
            Candidate::CumulativeLink(link) => MLModel::OrdinalLogistic(ModelWithMeta {
                model: OrdinalLogistic::<f64>::params()
                    .link(link)
                    .loss(OrdinalLoss::CumulativeLink)
                    .n_levels(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            Candidate::AllThreshold(margin) => MLModel::OrdinalLogistic(ModelWithMeta {
                model: OrdinalLogistic::<f64>::params()
                    .loss(OrdinalLoss::AllThreshold)
                    .margin(margin)
                    .n_levels(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            Candidate::Ridge(alpha) => MLModel::OrdinalRidge(ModelWithMeta {
                model: OrdinalRidge::<f64>::params()
                    .alpha(alpha)
                    .n_levels(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            Candidate::ContinuationRatio => MLModel::OrdinalContinuationRatio(ModelWithMeta {
                model: ContinuationRatio::<f64>::params()
                    .n_levels(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            Candidate::AdjacentCategory => MLModel::OrdinalAdjacentCategory(ModelWithMeta {
                model: AdjacentCategory::<f64>::params()
                    .n_levels(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            Candidate::Neural(head) => MLModel::OrdinalNeural(ModelWithMeta {
                model: neural_params(head, n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
        };
        Ok(model)
    }
}

/// Running state of one candidate over the folds.
#[cfg(feature = "execute")]
struct Trial {
    candidate: Candidate,
    fold_scores: Vec<f64>,
    elapsed_secs: f64,
    failure: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct AutoOrdinalNode {}

impl AutoOrdinalNode {
    pub fn new() -> Self {
        AutoOrdinalNode {}
    }
}

#[async_trait]
impl NodeLogic for AutoOrdinalNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_tuning_auto_ordinal",
            "Auto Ordinal",
            "Automatically finds the best model for a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Cross-validates the ordinal families - Proportional Odds and Ordered Probit, the all-threshold model and its support-vector form, Ordinal Ridge, Continuation Ratio and Adjacent Category, plus an optional rank-consistent neural family that is off by default because it costs far more than all the others combined - on identical folds, ranks them by an ordinal metric that knows how far a miss was, then retrains the winner on the full data. Use this rather than Auto Classifier, which resolves the target without its order and ranks by accuracy or macro-F1, scoring a five-level miss exactly like a one-level one. Every candidate here is a gradient or a least-squares fit on the raw columns, so scale your features with the Fit Feature Scaler node first: unscaled columns change which family wins, not just how fast it converges.",
            "AI/ML/Tuning",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(3) // Fits every family on every fold
                .set_governance(8)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts the ordinal model search",
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
            "class_order",
            "Class Order",
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so the search fails rather than guessing one. The level set is resolved once here and handed to every family, so a level that a fold happens to miss cannot renumber the ranks for that fold.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "cv_folds",
            "CV Folds",
            "How many folds the rows are split into. Every family is scored on the SAME folds, so the comparison is paired rather than a race between different splits. More folds mean a less noisy score and proportionally more fitting, since the whole sweep is repeated once per fold.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((2.0, 50.0)).build())
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "metric",
            "Metric",
            "What the leaderboard is ranked by. Quadratic Kappa is chance-corrected agreement that forgives a near miss and punishes a distant one four times as hard - the standard headline metric for ordered targets. Linear Kappa charges the same for every step along the scale, which is what you want when one level is one unit of loss. Mean Rank Error is the average number of levels a prediction is off by, and Macro Rank Error is the same averaged per true level so a rare level counts as much as the majority one - both are ERROR metrics, so the leaderboard ranks their smallest value first. Kendall Tau-b and Spearman ask only whether the rows come out in the right order and ignore calibration entirely, so a model whose levels are all shifted by one still scores perfectly.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(METRICS.iter().map(|metric| metric.to_string()).collect())
                .build(),
        )
        .set_default_value(Some(json!(METRIC_QUADRATIC_KAPPA)));

        node.add_input_pin(
            "seed",
            "Seed",
            "Seed for the fold shuffle. The same seed reproduces the same folds and therefore the same leaderboard; change it to check whether a narrow win survives a different split.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0.0, 4294967295.0)).build())
        .set_default_value(Some(json!(42)));

        node.add_input_pin(
            "include_proportional_odds",
            "Include Proportional Odds",
            "Try the cumulative-link model under a logit and a probit link. The only family here that yields calibrated per-level probabilities and coefficients that read as a direction along the ordering, but it assumes one shared effect across all cut points.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_all_threshold",
            "Include All-Threshold",
            "Try the all-threshold model under a logistic and a hinge margin. It drops the proportional-odds assumption by fitting cut-point placement instead of a likelihood, which is often more robust when that assumption fails; the hinge entry is support vector ordinal regression. Neither yields per-level probabilities.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_ordinal_ridge",
            "Include Ordinal Ridge",
            "Try rank regression with learned cut points across a small L2 sweep. Closed-form, so it is by far the cheapest candidate and stays cheap as levels and features grow - but it treats the ranks as numbers, so it is the family most likely to be beaten when the levels are not evenly spaced.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_continuation_ratio",
            "Include Continuation Ratio",
            "Try the sequential model, `P(stop at level k | reached level k)`. The right shape when reaching a level genuinely requires passing the ones below it (stages, escalation, dropout). It fits K-1 sub-models on shrinking subsets and refuses to fit at all when a middle level is missing from a fold, in which case it is dropped from the leaderboard and the other families continue.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_adjacent_category",
            "Include Adjacent Category",
            "Try the adjacent-category model, which contrasts neighbouring levels instead of splitting the scale cumulatively. Reach for it when the interesting comparison is `this level versus the next one` rather than `at most this level versus above it`.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_neural",
            "Include Neural",
            "Try a small neural network under a rank-consistent head, as two candidates: a CORAL head, which shares one latent score across the cut points and lets them differ only by biases that cannot cross, and a CORN head, which fits one conditional task per cut point on the rows that reached it. OFF by default, unlike every other family here, and the default is the recommendation: a network is orders of magnitude more expensive to fit than the linear families, it is refitted from scratch on EVERY fold, and it is the one candidate that can dominate the runtime of the whole sweep. Switch it on when you suspect the levels are not separated by a single monotone direction in the features - the hidden layer is the entire contribution, and it is the only thing here that can represent such a boundary at all. On a problem that is linear in the features it can only match the simpler families, never beat them: with no hidden layer CORAL is EXACTLY the all-threshold model with a logistic margin and CORN is EXACTLY Continuation Ratio, so prefer those better-tested candidates when they win. Both use a fixed initialization seed, so the leaderboard stays reproducible. CORN is dropped from the leaderboard on any fold that omits a level nothing reaches, since its task for that level would have no rows; CORAL has no such failure mode.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the search completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Leaderboard of every configuration that finished, best first, plus the ones that were dropped and why. `higher_is_better` states which end of `cv_score` won.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalAutoMLResult>();

        node.add_output_pin(
            "best_model",
            "Best Model",
            "The winning configuration retrained on the full dataset. Predictions come back as your original level labels.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "best_model_type",
            "Best Model Type",
            "Model kind of the winner, e.g. `OrdinalLogistic`. Read back off the retrained model, so it always matches what the rest of the catalog calls it.",
            VariableType::String,
        );

        node.add_output_pin(
            "levels",
            "Levels",
            "The level order every candidate was trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when the leaderboard looks upside down.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalLevels>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let cv_folds: i64 = context.evaluate_pin("cv_folds").await?;
        let metric_name: String = context.evaluate_pin("metric").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;
        let include_proportional_odds: bool =
            context.evaluate_pin("include_proportional_odds").await?;
        let include_all_threshold: bool = context.evaluate_pin("include_all_threshold").await?;
        let include_ordinal_ridge: bool = context.evaluate_pin("include_ordinal_ridge").await?;
        let include_continuation_ratio: bool =
            context.evaluate_pin("include_continuation_ratio").await?;
        let include_adjacent_category: bool =
            context.evaluate_pin("include_adjacent_category").await?;
        // Boards placed before this pin existed sweep exactly the families they always did: the
        // fallback is the pin's own default, so nothing about an older board's leaderboard moves.
        let include_neural: bool = context
            .evaluate_pin("include_neural")
            .await
            .unwrap_or(false);

        let metric = OrdinalMetric::parse(&metric_name)?;
        // Checked before the cast: a negative value wraps to usize::MAX, sails past every later
        // comparison and blows up in Vec::with_capacity.
        if cv_folds < 2 {
            return Err(anyhow!(
                "`CV Folds` must be at least 2, got {cv_folds}. One fold leaves nothing to validate on."
            ));
        }
        let cv_folds = cv_folds as usize;
        if seed < 0 {
            return Err(anyhow!("`Seed` must not be negative, got {seed}"));
        }
        let seed = seed as u64;

        let mut candidates: Vec<Candidate> = Vec::new();
        if include_proportional_odds {
            candidates.push(Candidate::CumulativeLink(Link::Logit));
            candidates.push(Candidate::CumulativeLink(Link::Probit));
        }
        if include_all_threshold {
            candidates.push(Candidate::AllThreshold(Margin::Logistic));
            candidates.push(Candidate::AllThreshold(Margin::Hinge));
        }
        if include_ordinal_ridge {
            candidates.extend(RIDGE_ALPHAS.iter().map(|alpha| Candidate::Ridge(*alpha)));
        }
        if include_continuation_ratio {
            candidates.push(Candidate::ContinuationRatio);
        }
        if include_adjacent_category {
            candidates.push(Candidate::AdjacentCategory);
        }
        if include_neural {
            candidates.push(Candidate::Neural(OrdinalHead::Coral));
            candidates.push(Candidate::Neural(OrdinalHead::Corn));
        }
        if candidates.is_empty() {
            return Err(anyhow!(
                "No model family is included, so there is nothing to compare. Switch on at least one of `Include Proportional Odds`, `Include All-Threshold`, `Include Ordinal Ridge`, `Include Continuation Ratio`, `Include Adjacent Category` or `Include Neural`."
            ));
        }

        let explicit_order: Vec<String> = class_order
            .split(',')
            .map(|level| level.trim())
            .filter(|level| !level.is_empty())
            .map(ToString::to_string)
            .collect();

        let start = Instant::now();
        let (features, ranks, classes, levels) = match source.as_str() {
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
                        return Err(anyhow!(
                            "Database doesn't contain train col `{}`!",
                            records_col
                        ));
                    }
                    if !existing_cols.contains(&targets_col) {
                        return Err(anyhow!(
                            "Database doesn't contain target col `{}`!",
                            targets_col
                        ));
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
                        "No training records in the database; an ordinal search needs at least one row"
                    ));
                }

                let features = values_to_array2_f64(&records, &records_col)?;
                let (ranks, classes, levels) = values_to_array1_ordinal(
                    &records,
                    &targets_col,
                    (!explicit_order.is_empty()).then_some(explicit_order.as_slice()),
                )?;
                (features, ranks, classes, levels)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };

        let (n_samples, n_features) = features.dim();
        if n_features == 0 {
            return Err(anyhow!(
                "Training records have 0 features, expected at least one value per row"
            ));
        }
        // Every candidate is either an Adam fit or a Cholesky solve, and both turn a single NaN into
        // an all-NaN model while only reporting that "the feature matrix" was non-finite.
        if let Some(((row, col), value)) = features
            .indexed_iter()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(anyhow!(
                "Training feature at row {row}, column {col} is {value}; ordinal fitting needs finite features. Clean or impute the column before searching."
            ));
        }
        if n_samples < cv_folds {
            return Err(anyhow!(
                "Cannot run {cv_folds}-fold cross-validation on {n_samples} rows. Reduce CV Folds or supply more training data."
            ));
        }

        let n_levels = levels.labels.len();
        let observed: HashSet<usize> = ranks.iter().copied().collect();
        if observed.len() < 2 {
            let seen = observed
                .iter()
                .filter_map(|rank| levels.labels.get(*rank))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow!(
                "Ordinal models need at least 2 distinct levels in the training data, but only [{seen}] occurs. Widen the training set or check the target column."
            ));
        }

        let ordering_source = match levels.ordering {
            OrdinalOrdering::Explicit => "from your Class Order list",
            OrdinalOrdering::Numeric => "inferred by reading the labels as numbers",
        };
        // A wrong order trains every candidate confidently backwards and nothing in the leaderboard
        // reveals it, so the resolved order goes to the run log rather than only to the output pin.
        context.log_message(
            &format!(
                "Ordinal level order ({ordering_source}): {}",
                levels.labels.join(" < ")
            ),
            LogLevel::Info,
        );
        context.log_message(
            &format!(
                "Auto Ordinal: {n_samples} samples x {n_features} features, {n_levels} levels, {} configurations, {cv_folds} folds, metric {}",
                candidates.len(),
                metric.as_str()
            ),
            LogLevel::Info,
        );
        // The runtime jump is large enough that a user who left this on by accident should be told
        // once, together with what the extra cost is actually buying.
        if include_neural {
            context.log_message(
                &format!(
                    "Neural candidates included: two networks with hidden layers [{}] are refitted on each of the {cv_folds} folds, which typically dominates the runtime of the whole sweep. Their hidden layer is the only thing they add - with no hidden layer CORAL is exactly the all-threshold model and CORN is exactly Continuation Ratio - so if a linear family still wins, prefer it: it is simpler, better tested and yields readable coefficients.",
                    neural_hidden_layer_summary()
                ),
                LogLevel::Info,
            );
        }

        let mut indices: Vec<usize> = (0..n_samples).collect();
        // Seeded on purpose: an unseeded shuffle makes the leaderboard irreproducible, and with
        // several families within noise of each other the winner would change between runs.
        let mut rng = StdRng::seed_from_u64(seed);
        indices.shuffle(&mut rng);
        let fold_size = n_samples / cv_folds;

        let mut trials: Vec<Trial> = candidates
            .into_iter()
            .map(|candidate| Trial {
                candidate,
                fold_scores: Vec::with_capacity(cv_folds),
                elapsed_secs: 0.0,
                failure: None,
            })
            .collect();

        // Folds outermost: every candidate sees the same split, and the split is materialised once
        // per fold instead of once per candidate and fold.
        let mut metric_direction: Option<bool> = None;
        for fold in 0..cv_folds {
            let (train, validation_features, validation_ranks) =
                split_fold(&features, &ranks, &indices, fold, fold_size, cv_folds);

            for trial in trials.iter_mut() {
                if trial.failure.is_some() {
                    continue;
                }
                let started = Instant::now();
                let outcome = trial
                    .candidate
                    .fit_predict(&train, &validation_features, n_levels)
                    .and_then(|predicted| {
                        score_predictions(metric, &predicted.to_vec(), &validation_ranks, n_levels)
                    });
                trial.elapsed_secs += started.elapsed().as_secs_f64();

                match outcome {
                    Ok((score, higher_is_better)) => {
                        metric_direction = Some(higher_is_better);
                        trial.fold_scores.push(score);
                    }
                    Err(err) => {
                        // One family's fold failure must not end the sweep: Continuation Ratio in
                        // particular refuses to fit when a fold omits a middle level, and the other
                        // families have nothing to do with that.
                        let reason = format!("fold {} of {cv_folds}: {err}", fold + 1);
                        context.log_message(
                            &format!(
                                "`{}` failed on {reason}. It is dropped from the leaderboard; the remaining configurations continue.",
                                trial.candidate.variant()
                            ),
                            LogLevel::Warn,
                        );
                        trial.failure = Some(reason);
                    }
                }
            }
        }

        let skipped: Vec<OrdinalAutoMLSkip> = trials
            .iter()
            .filter_map(|trial| {
                trial.failure.as_ref().map(|reason| OrdinalAutoMLSkip {
                    variant: trial.candidate.variant(),
                    reason: reason.clone(),
                })
            })
            .collect();

        let Some(higher_is_better) = metric_direction else {
            let reasons = skipped
                .iter()
                .map(|skip| format!("{} ({})", skip.variant, skip.reason))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(anyhow!(
                "Every included configuration failed to fit, so there is no leaderboard to rank. Failures: {reasons}"
            ));
        };

        let mut ranked: Vec<(Candidate, OrdinalAutoMLEntry)> = trials
            .into_iter()
            .filter(|trial| trial.failure.is_none() && !trial.fold_scores.is_empty())
            .map(|trial| {
                let cv_score =
                    trial.fold_scores.iter().sum::<f64>() / trial.fold_scores.len() as f64;
                (
                    trial.candidate,
                    OrdinalAutoMLEntry {
                        model_type: trial.candidate.model_type().to_string(),
                        variant: trial.candidate.variant(),
                        params: trial.candidate.params(),
                        cv_score,
                        train_time_secs: trial.elapsed_secs,
                        rank: 0,
                    },
                )
            })
            .collect();

        // Sorted through the metric's own direction rather than always descending, which is what
        // keeps an error metric from ranking its worst model first.
        ranked.sort_by(|(_, left), (_, right)| {
            let ordering = if higher_is_better {
                right.cv_score.partial_cmp(&left.cv_score)
            } else {
                left.cv_score.partial_cmp(&right.cv_score)
            };
            ordering.unwrap_or(std::cmp::Ordering::Equal)
        });
        for (position, (_, entry)) in ranked.iter_mut().enumerate() {
            entry.rank = position + 1;
        }

        for (_, entry) in ranked.iter() {
            context.log_message(
                &format!(
                    "#{} {} [{}]: {} = {:.4} in {:.2}s",
                    entry.rank,
                    entry.variant,
                    entry.model_type,
                    metric.as_str(),
                    entry.cv_score,
                    entry.train_time_secs
                ),
                LogLevel::Info,
            );
        }

        let best_candidate = ranked
            .first()
            .map(|(candidate, _)| *candidate)
            .ok_or_else(|| anyhow!("Leaderboard is empty after ranking"))?;

        let dataset = DatasetBase::new(features, ranks);
        let final_model = best_candidate.fit_final(&dataset, n_levels, classes)?;
        let best_model_type = final_model.kind().to_string();

        let leaderboard: Vec<OrdinalAutoMLEntry> =
            ranked.into_iter().map(|(_, entry)| entry).collect();
        let result = OrdinalAutoMLResult {
            // Derived, never counted by hand: a hardcoded total drifts the moment a family is
            // switched off or dropped after a failed fold.
            total_models_tried: leaderboard.len(),
            leaderboard,
            skipped,
            best_index: 0,
            total_time_secs: start.elapsed().as_secs_f64(),
            metric: metric.as_str().to_string(),
            higher_is_better,
            n_levels,
            n_folds: cv_folds,
            n_samples,
        };

        context.log_message(
            &format!(
                "Auto Ordinal complete: `{}` wins with {} = {:.4} ({} configurations ranked, {} dropped) in {:.2}s",
                result.leaderboard[0].variant,
                result.metric,
                result.leaderboard[0].cv_score,
                result.total_models_tried,
                result.skipped.len(),
                result.total_time_secs
            ),
            LogLevel::Info,
        );

        let node_model = NodeMLModel::new(context, final_model).await;
        context.set_pin_value("results", json!(result)).await?;
        context
            .set_pin_value("best_model", json!(node_model))
            .await?;
        context
            .set_pin_value("best_model_type", json!(best_model_type))
            .await?;
        context.set_pin_value("levels", json!(levels)).await?;
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
                    "Column Containing the Feature Vectors to Train on",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("targets").is_none() {
                node.add_input_pin(
                    "targets",
                    "Target Col",
                    "Column Containing the Ordered Level of each Row",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}

/// Materialises one cross-validation split from the shuffled row order.
#[cfg(feature = "execute")]
fn split_fold(
    features: &Array2<f64>,
    ranks: &Array1<usize>,
    indices: &[usize],
    fold: usize,
    fold_size: usize,
    cv_folds: usize,
) -> (
    DatasetBase<Array2<f64>, Array1<usize>>,
    Array2<f64>,
    Vec<usize>,
) {
    let validation_start = fold * fold_size;
    // The last fold absorbs the remainder, so no row is dropped when the row count is not divisible
    // by the fold count.
    let validation_end = if fold == cv_folds - 1 {
        indices.len()
    } else {
        validation_start + fold_size
    };

    let validation_rows = &indices[validation_start..validation_end];
    let training_rows: Vec<usize> = indices
        .iter()
        .enumerate()
        .filter(|(position, _)| *position < validation_start || *position >= validation_end)
        .map(|(_, row)| *row)
        .collect();

    let training_features = features.select(Axis(0), &training_rows);
    let training_ranks: Array1<usize> = training_rows.iter().map(|row| ranks[*row]).collect();
    let validation_features = features.select(Axis(0), validation_rows);
    let validation_ranks: Vec<usize> = validation_rows.iter().map(|row| ranks[*row]).collect();

    (
        DatasetBase::new(training_features, training_ranks),
        validation_features,
        validation_ranks,
    )
}
