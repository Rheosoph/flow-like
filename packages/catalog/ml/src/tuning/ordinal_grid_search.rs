//! Grid search for ONE **ordered** target model family.
//!
//! The nominal Grid Search node cannot stand in for this, and the difference is not cosmetic. It
//! resolves the target with `values_to_array1_target`, which assigns rank ids in whatever order the
//! labels happen to appear and therefore throws the level ORDER away, and it scores every candidate
//! by accuracy, which charges the same for predicting `low` instead of `high` as for predicting
//! `medium` instead of `high`. Tuning a threshold model against that signal optimises the wrong
//! thing twice over. So this node keeps the ordinal contract end to end: the target is resolved
//! into ranks with a declared level order, every configuration is fitted against the same declared
//! level count, and the sweep is ranked by a distance-aware ordinal metric.
//!
//! It is the exhaustive counterpart to Auto Ordinal. Auto Ordinal compares the *families* at
//! sensible defaults; this node takes one family and searches its hyperparameters properly.

use crate::ml::{GridSearchEntry, NodeMLModel, OrdinalLevels, ParameterSpec};
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, OrdinalOrdering, values_to_array1_ordinal,
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
use flow_like_ordinal::{
    Activation, AdjacentCategory, AdjacentCategoryParams, ContinuationRatio,
    ContinuationRatioParams, Link, Margin, OrdinalError, OrdinalHead, OrdinalLogistic,
    OrdinalLogisticParams, OrdinalLoss, OrdinalNeural, OrdinalNeuralParams, OrdinalRidge,
    OrdinalRidgeParams, kendall_tau_b, linear_weighted_kappa, macro_mean_absolute_error,
    mean_absolute_rank_error, quadratic_weighted_kappa, spearman_rank_correlation,
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

/// A parameter combination that was dropped from the sweep because it could not be fitted.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalGridSearchSkip {
    /// The combination that failed, as it appeared in the grid.
    pub params: HashMap<String, Value>,
    /// Which fold it failed on and what the estimator reported.
    pub reason: String,
}

/// Complete ordinal grid search results.
///
/// A local struct rather than `crate::ml::GridSearchResult` because a ranking over an ordinal
/// metric is not self-describing without its direction: two of the six metrics are errors, where
/// the SMALLEST score wins. Reading `best_score` off a result that cannot say which end it came
/// from is exactly how a tuned model turns out to be the worst one tried. The per-combination
/// entries are the shared [`GridSearchEntry`], so the two searches report a combination
/// identically.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalGridSearchResult {
    /// Every combination that completed all folds, in the order the grid produced them.
    pub results: Vec<GridSearchEntry>,
    /// Combinations excluded because a fold could not be fitted, with the reason.
    pub skipped: Vec<OrdinalGridSearchSkip>,
    /// Index of the winning entry inside `results`.
    pub best_index: usize,
    /// Hyperparameters of the winner.
    pub best_params: HashMap<String, Value>,
    /// Mean cross-validated score of the winner, in the units of `metric`.
    pub best_score: f64,
    /// Model family that was tuned, e.g. `OrdinalLogistic`.
    pub model_type: String,
    /// Metric the sweep was ranked by.
    pub metric: String,
    /// False for the two error metrics, where the SMALLEST `mean_score` is the winner.
    pub higher_is_better: bool,
    /// Wall clock seconds for the whole sweep, including data loading and the final refit.
    pub total_time_secs: f64,
    /// Size of the cartesian product of the grid. Combinations counted in `skipped` are included
    /// here but absent from `results`.
    pub n_combinations: usize,
    /// Number of cross-validation folds.
    pub n_folds: usize,
    /// Number of ordered levels every configuration was fitted against.
    pub n_levels: usize,
    /// Rows the sweep ran on.
    pub n_samples: usize,
}

/// Model kinds this node can tune, mirroring `MLModel::kind()` in `crate::ml`.
///
/// These are the same strings Auto Ordinal reports as `best_model_type`, so that node's output
/// feeds straight into this node's `model_type` input: compare the families there, then tune the
/// winner here.
const KIND_LOGISTIC: &str = "OrdinalLogistic";
const KIND_RIDGE: &str = "OrdinalRidge";
const KIND_CONTINUATION_RATIO: &str = "OrdinalContinuationRatio";
const KIND_ADJACENT_CATEGORY: &str = "OrdinalAdjacentCategory";
const KIND_NEURAL: &str = "OrdinalNeural";

const TUNABLE_ORDINAL_MODELS: [&str; 5] = [
    KIND_LOGISTIC,
    KIND_RIDGE,
    KIND_CONTINUATION_RATIO,
    KIND_ADJACENT_CATEGORY,
    KIND_NEURAL,
];

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

#[cfg(feature = "execute")]
const LINK_NAMES: [&str; 4] = ["Logit", "Probit", "CLogLog", "Cauchit"];
#[cfg(feature = "execute")]
const LOSS_NAMES: [&str; 3] = ["CumulativeLink", "AllThreshold", "ImmediateThreshold"];
#[cfg(feature = "execute")]
const MARGIN_NAMES: [&str; 3] = ["Logistic", "Hinge", "SquaredHinge"];
#[cfg(feature = "execute")]
const HEAD_NAMES: [&str; 2] = ["Coral", "Corn"];
#[cfg(feature = "execute")]
const ACTIVATION_NAMES: [&str; 2] = ["Relu", "Tanh"];

/// The metric the sweep is ranked by.
///
/// Mirrors the metric set of the Auto Ordinal node so a family chosen there and tuned here is
/// judged by the same number.
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
/// wins. Ranking an error metric the wrong way round crowns the *worst* combination in the grid and
/// leaves a result that looks entirely plausible, so the value and its direction are produced by
/// one match and handed out as a pair: no caller can obtain a score without also being handed the
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

/// The ordinal family being tuned.
#[cfg(feature = "execute")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrdinalFamily {
    Logistic,
    Ridge,
    ContinuationRatio,
    AdjacentCategory,
    Neural,
}

#[cfg(feature = "execute")]
impl OrdinalFamily {
    fn parse(model_type: &str) -> Result<Self> {
        match model_type {
            KIND_LOGISTIC => Ok(OrdinalFamily::Logistic),
            KIND_RIDGE => Ok(OrdinalFamily::Ridge),
            KIND_CONTINUATION_RATIO => Ok(OrdinalFamily::ContinuationRatio),
            KIND_ADJACENT_CATEGORY => Ok(OrdinalFamily::AdjacentCategory),
            KIND_NEURAL => Ok(OrdinalFamily::Neural),
            other => Err(anyhow!(
                "Unknown model type: `{other}`. Supported: {}",
                TUNABLE_ORDINAL_MODELS.join(", ")
            )),
        }
    }

    /// Parameter names this family actually consumes.
    ///
    /// The Parameter Grid pin is seeded once from whichever model type was selected when the node
    /// was created and is deliberately never rewritten (that would clobber a hand-edited grid).
    /// Without this list, switching Model Type afterwards would leave the previous family's
    /// parameters in place; every one of them would be ignored by the builder, and the sweep would
    /// score N identical configurations and report the last one as a tuned result.
    fn known_params(self) -> &'static [&'static str] {
        match self {
            OrdinalFamily::Logistic => &[
                "alpha",
                "link",
                "loss",
                "margin",
                "learning_rate",
                "max_iterations",
            ],
            OrdinalFamily::Ridge => &["alpha"],
            OrdinalFamily::ContinuationRatio => &["alpha", "link", "learning_rate"],
            OrdinalFamily::AdjacentCategory => &["alpha", "learning_rate"],
            OrdinalFamily::Neural => &[
                "alpha",
                "head",
                "activation",
                "hidden_layers",
                "learning_rate",
                "max_iterations",
                "seed",
            ],
        }
    }
}

/// Default parameter grid for a family, used to seed the Parameter Grid pin.
///
/// Kept small on purpose: every entry multiplies into the cartesian product, and each resulting
/// configuration is refitted once per fold.
#[cfg(feature = "execute")]
fn default_param_grid(model_type: &str) -> Value {
    match OrdinalFamily::parse(model_type) {
        // Deliberately NOT sweeping `link` here: Auto Ordinal already decides Logit vs Probit when
        // it picks the family, so re-deciding it downstream spends the budget on a settled question
        // while leaving the Adam step size — which decides whether the fit converges at all —
        // untouched. Same combination count either way.
        Ok(OrdinalFamily::Logistic) => json!([
            {"name": "alpha", "values": [0.1, 1.0, 10.0]},
            {"name": "learning_rate", "values": [0.05, 0.1]}
        ]),
        Ok(OrdinalFamily::Ridge) => json!([
            {"name": "alpha", "values": [0.01, 0.1, 1.0, 10.0]}
        ]),
        Ok(OrdinalFamily::ContinuationRatio) => json!([
            {"name": "alpha", "values": [0.1, 1.0, 10.0]},
            {"name": "learning_rate", "values": [0.05, 0.1]}
        ]),
        Ok(OrdinalFamily::AdjacentCategory) => json!([
            {"name": "alpha", "values": [0.1, 1.0, 10.0]},
            {"name": "learning_rate", "values": [0.05, 0.1]}
        ]),
        // Two dimensions, and no more. The backbone is the point of this family — the estimator's
        // own width, twice that, and whether a second layer helps at all — but architecture alone
        // is not enough: this is the only non-convex estimator in the node, and a step size that
        // does not converge fails all three architectures identically, so the sweep would report
        // nothing rather than a winner. Everything else (alpha, head, activation) stays out because
        // each entry multiplies into the cartesian product and every combination trains a network
        // from scratch on each fold.
        Ok(OrdinalFamily::Neural) => json!([
            {"name": "hidden_layers", "values": ["16", "32", "16, 8"]},
            {"name": "learning_rate", "values": [0.01, 0.05]}
        ]),
        Err(_) => json!([]),
    }
}

/// One parsed grid combination, ready to fit.
///
/// Every grid field is optional so that a parameter absent from the grid keeps the estimator's own
/// default instead of being silently pinned to a value chosen here.
///
/// NOT `Copy`: the neural backbone is a `Vec<usize>` of arbitrary depth, and the alternative -
/// pinning it to a fixed-capacity array to keep the marker - would cap how deep a network the user
/// may write in the grid for the sake of one dereference on the winning combination. The only site
/// that relied on `Copy` is the winner's refit, which clones instead.
#[cfg(feature = "execute")]
#[derive(Debug, Clone)]
struct OrdinalConfig {
    family: OrdinalFamily,
    alpha: Option<f64>,
    learning_rate: Option<f64>,
    max_iterations: Option<usize>,
    link: Option<Link>,
    loss: Option<OrdinalLoss>,
    margin: Option<Margin>,
    head: Option<OrdinalHead>,
    activation: Option<Activation>,
    hidden_layers: Option<Vec<usize>>,
    /// Weight initialization seed for the neural family, taken from the node's Seed pin. Not a grid
    /// parameter: it is carried on the config so the winner's refit initializes from exactly the
    /// same point the cross-validated score was earned at.
    seed: u64,
}

/// Reads a JSON value as an integer, accepting `500` and `500.0` alike.
///
/// Grids are hand-edited JSON, and a value typed with a decimal point is not a different intent.
#[cfg(feature = "execute")]
fn as_integer(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && number.fract() == 0.0)
            .map(|number| number as i64)
    })
}

#[cfg(feature = "execute")]
impl OrdinalConfig {
    /// Parses one combination of the cartesian product.
    ///
    /// Runs before any data is loaded so a malformed grid entry fails immediately rather than after
    /// the first fold of the first combination has already been fitted.
    ///
    /// `seed` is the node's Seed pin, reused for the neural weight initialization; see the field.
    fn parse(family: OrdinalFamily, params: &HashMap<String, Value>, seed: u64) -> Result<Self> {
        let mut config = OrdinalConfig {
            family,
            alpha: None,
            learning_rate: None,
            max_iterations: None,
            link: None,
            loss: None,
            margin: None,
            head: None,
            activation: None,
            hidden_layers: None,
            seed,
        };

        if let Some(value) = params.get("alpha") {
            let alpha = value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| anyhow!("Parameter `alpha` expects a finite number, got {value}"))?;
            // Ridge solves penalized normal equations, and the penalty is what keeps them
            // invertible, so zero is rejected here rather than as a fit failure on every fold.
            let lower_bound_violated = match family {
                OrdinalFamily::Ridge => alpha <= 0.0,
                _ => alpha < 0.0,
            };
            if lower_bound_violated {
                return Err(anyhow!(
                    "Parameter `alpha` is {alpha}, but {} needs {}.",
                    family_label(family),
                    match family {
                        OrdinalFamily::Ridge =>
                            "a strictly positive penalty to keep the normal equations solvable",
                        _ => "a non-negative penalty",
                    }
                ));
            }
            config.alpha = Some(alpha);
        }

        if let Some(value) = params.get("learning_rate") {
            let rate = value
                .as_f64()
                .filter(|number| number.is_finite() && *number > 0.0)
                .ok_or_else(|| {
                    anyhow!(
                        "Parameter `learning_rate` expects a finite positive number, got {value}"
                    )
                })?;
            config.learning_rate = Some(rate);
        }

        if let Some(value) = params.get("max_iterations") {
            // Checked before the cast: a negative value wraps to a gigantic usize, which the
            // estimator accepts and then spends the rest of the day on.
            let iterations = as_integer(value)
                .filter(|number| *number >= 1)
                .ok_or_else(|| {
                    anyhow!("Parameter `max_iterations` expects a whole number of at least 1, got {value}")
                })?;
            config.max_iterations = Some(iterations as usize);
        }

        if let Some(value) = params.get("link") {
            config.link = Some(parse_named(
                value,
                "link",
                &LINK_NAMES,
                |name| match name {
                    "Logit" => Some(Link::Logit),
                    "Probit" => Some(Link::Probit),
                    "CLogLog" => Some(Link::CLogLog),
                    "Cauchit" => Some(Link::Cauchit),
                    _ => None,
                },
            )?);
        }

        if let Some(value) = params.get("loss") {
            config.loss = Some(parse_named(
                value,
                "loss",
                &LOSS_NAMES,
                |name| match name {
                    "CumulativeLink" => Some(OrdinalLoss::CumulativeLink),
                    "AllThreshold" => Some(OrdinalLoss::AllThreshold),
                    "ImmediateThreshold" => Some(OrdinalLoss::ImmediateThreshold),
                    _ => None,
                },
            )?);
        }

        if let Some(value) = params.get("margin") {
            config.margin = Some(parse_named(
                value,
                "margin",
                &MARGIN_NAMES,
                |name| match name {
                    "Logistic" => Some(Margin::Logistic),
                    "Hinge" => Some(Margin::Hinge),
                    "SquaredHinge" => Some(Margin::SquaredHinge),
                    _ => None,
                },
            )?);
        }

        if let Some(value) = params.get("head") {
            config.head = Some(parse_named(
                value,
                "head",
                &HEAD_NAMES,
                |name| match name {
                    "Coral" => Some(OrdinalHead::Coral),
                    "Corn" => Some(OrdinalHead::Corn),
                    _ => None,
                },
            )?);
        }

        if let Some(value) = params.get("activation") {
            config.activation = Some(parse_named(
                value,
                "activation",
                &ACTIVATION_NAMES,
                |name| match name {
                    "Relu" => Some(Activation::Relu),
                    "Tanh" => Some(Activation::Tanh),
                    _ => None,
                },
            )?);
        }

        // A string rather than a JSON array, so the grid says what the neural node's Hidden Layers
        // pin says and a user can move a value between the two unchanged. Parsed exactly as that
        // pin parses it, down to skipping blank entries, so a grid that trains there trains here.
        if let Some(value) = params.get("hidden_layers") {
            let mut widths: Vec<usize> = Vec::new();
            // Two accepted spellings on purpose. The string form matches the neural node's Hidden
            // Layers pin, so a value moves between the two unchanged. The array form is what Auto
            // Ordinal writes into its leaderboard, so a winning row can be pasted straight into
            // this grid — rejecting it would break the very hand-off this node exists to serve.
            let tokens: Vec<String> = match value {
                Value::String(text) => text
                    .split(',')
                    .map(|token| token.trim().to_string())
                    .filter(|token| !token.is_empty())
                    .collect(),
                Value::Array(entries) => entries.iter().map(|entry| entry.to_string()).collect(),
                other => {
                    return Err(anyhow!(
                        "Parameter `hidden_layers` expects comma-separated layer widths as a string such as \"16, 8\", or a list such as [16, 8], got {other}. Use \"\" for no hidden layer at all."
                    ));
                }
            };
            for token in tokens {
                let width = token.parse::<usize>().map_err(|_| {
                    anyhow!(
                        "Parameter `hidden_layers` takes comma-separated layer widths such as `16, 8`, but `{token}` is not a whole number. Use an empty string for no hidden layer at all."
                    )
                })?;
                if width == 0 {
                    return Err(anyhow!(
                        "Parameter `hidden_layers` gives layer {} a width of 0 (entry `{token}`); a zero-width layer disconnects the head from the features and can only fit a constant. Give it at least 1 unit, or use an empty string for no hidden layer at all.",
                        widths.len()
                    ));
                }
                widths.push(width);
            }
            config.hidden_layers = Some(widths);
        }

        // Also accepted so an Auto Ordinal leaderboard row pastes in whole. Sweeping it is a real
        // use too: a win that survives several seeds is a win, and one that does not was luck in
        // the weight initialization.
        if let Some(value) = params.get("seed") {
            let seed = as_integer(value).ok_or_else(|| {
                anyhow!("Parameter `seed` expects a non-negative whole number, got {value}")
            })?;
            config.seed = seed as u64;
        }

        Ok(config)
    }

    fn logistic_params(&self, n_levels: usize) -> OrdinalLogisticParams<f64> {
        let mut params = OrdinalLogistic::<f64>::params().n_levels(n_levels);
        if let Some(alpha) = self.alpha {
            params = params.alpha(alpha);
        }
        if let Some(rate) = self.learning_rate {
            params = params.learning_rate(rate);
        }
        if let Some(iterations) = self.max_iterations {
            params = params.max_iterations(iterations);
        }
        if let Some(link) = self.link {
            params = params.link(link);
        }
        if let Some(loss) = self.loss {
            params = params.loss(loss);
        }
        if let Some(margin) = self.margin {
            params = params.margin(margin);
        }
        params
    }

    fn ridge_params(&self, n_levels: usize) -> OrdinalRidgeParams<f64> {
        let mut params = OrdinalRidge::<f64>::params().n_levels(n_levels);
        if let Some(alpha) = self.alpha {
            params = params.alpha(alpha);
        }
        params
    }

    fn continuation_params(&self, n_levels: usize) -> ContinuationRatioParams<f64> {
        let mut params = ContinuationRatio::<f64>::params().n_levels(n_levels);
        if let Some(alpha) = self.alpha {
            params = params.alpha(alpha);
        }
        if let Some(rate) = self.learning_rate {
            params = params.learning_rate(rate);
        }
        if let Some(link) = self.link {
            params = params.link(link);
        }
        params
    }

    fn adjacent_params(&self, n_levels: usize) -> AdjacentCategoryParams<f64> {
        let mut params = AdjacentCategory::<f64>::params().n_levels(n_levels);
        if let Some(alpha) = self.alpha {
            params = params.alpha(alpha);
        }
        if let Some(rate) = self.learning_rate {
            params = params.learning_rate(rate);
        }
        params
    }

    /// The neural configuration, built in one place so cross-validation and the winner's refit
    /// cannot diverge: the weight initialization is the only randomness in the fit, and a winner
    /// rebuilt from a different starting point is not the model whose score was reported.
    fn neural_params(&self, n_levels: usize) -> OrdinalNeuralParams<f64> {
        let mut params = OrdinalNeural::<f64>::params()
            .n_levels(n_levels)
            .seed(self.seed);
        if let Some(alpha) = self.alpha {
            params = params.alpha(alpha);
        }
        if let Some(rate) = self.learning_rate {
            params = params.learning_rate(rate);
        }
        if let Some(iterations) = self.max_iterations {
            params = params.max_iterations(iterations);
        }
        if let Some(head) = self.head {
            params = params.head(head);
        }
        if let Some(activation) = self.activation {
            params = params.activation(activation);
        }
        if let Some(layers) = self.hidden_layers.as_deref() {
            params = params.hidden_layers(layers);
        }
        params
    }

    /// Fits on the training split and predicts the held-out rows.
    ///
    /// `n_levels` is always the resolved level count, never the fold's own: a level a fold happens
    /// to miss would otherwise renumber the ranks for that fold alone and make its score
    /// incomparable with the others.
    fn fit_predict(
        &self,
        train: &DatasetBase<Array2<f64>, Array1<usize>>,
        validation: &Array2<f64>,
        n_levels: usize,
    ) -> Result<Array1<usize>> {
        let describe = |err: OrdinalError| anyhow!("{err}");
        let predicted = match self.family {
            OrdinalFamily::Logistic => self
                .logistic_params(n_levels)
                .fit(train)
                .map_err(describe)?
                .predict(validation),
            OrdinalFamily::Ridge => self
                .ridge_params(n_levels)
                .fit(train)
                .map_err(describe)?
                .predict(validation),
            OrdinalFamily::ContinuationRatio => self
                .continuation_params(n_levels)
                .fit(train)
                .map_err(describe)?
                .predict(validation),
            OrdinalFamily::AdjacentCategory => self
                .adjacent_params(n_levels)
                .fit(train)
                .map_err(describe)?
                .predict(validation),
            OrdinalFamily::Neural => self
                .neural_params(n_levels)
                .fit(train)
                .map_err(describe)?
                .predict(validation),
        };
        Ok(predicted)
    }

    /// Refits the winning combination on the full dataset and wraps it as a catalog model.
    fn fit_final(
        &self,
        dataset: &DatasetBase<Array2<f64>, Array1<usize>>,
        n_levels: usize,
        classes: HashMap<usize, String>,
        params: &HashMap<String, Value>,
    ) -> Result<MLModel> {
        let classes = Some(classes);
        let describe = |err: OrdinalError| {
            anyhow!(
                "`{}` won the search but failed to refit on the full dataset: {err}",
                describe_params(params)
            )
        };
        let model = match self.family {
            OrdinalFamily::Logistic => MLModel::OrdinalLogistic(ModelWithMeta {
                model: self
                    .logistic_params(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            OrdinalFamily::Ridge => MLModel::OrdinalRidge(ModelWithMeta {
                model: self.ridge_params(n_levels).fit(dataset).map_err(describe)?,
                classes,
            }),
            OrdinalFamily::ContinuationRatio => MLModel::OrdinalContinuationRatio(ModelWithMeta {
                model: self
                    .continuation_params(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            OrdinalFamily::AdjacentCategory => MLModel::OrdinalAdjacentCategory(ModelWithMeta {
                model: self
                    .adjacent_params(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
            OrdinalFamily::Neural => MLModel::OrdinalNeural(ModelWithMeta {
                model: self
                    .neural_params(n_levels)
                    .fit(dataset)
                    .map_err(describe)?,
                classes,
            }),
        };
        Ok(model)
    }
}

/// Human-readable name of a family, for error messages.
#[cfg(feature = "execute")]
fn family_label(family: OrdinalFamily) -> &'static str {
    match family {
        OrdinalFamily::Logistic => "the ordinal threshold model",
        OrdinalFamily::Ridge => "ordinal ridge",
        OrdinalFamily::ContinuationRatio => "the continuation ratio model",
        OrdinalFamily::AdjacentCategory => "the adjacent category model",
        OrdinalFamily::Neural => "the neural ordinal model",
    }
}

/// Resolves a string-valued grid entry to an enum, naming the accepted values on failure.
#[cfg(feature = "execute")]
fn parse_named<T>(
    value: &Value,
    parameter: &str,
    accepted: &[&str],
    resolve: impl Fn(&str) -> Option<T>,
) -> Result<T> {
    let text = value.as_str().ok_or_else(|| {
        anyhow!(
            "Parameter `{parameter}` expects one of {} as a string, got {value}",
            accepted.join(", ")
        )
    })?;
    resolve(text).ok_or_else(|| {
        anyhow!(
            "Unknown `{parameter}` value `{text}`. Accepted: {}",
            accepted.join(", ")
        )
    })
}

/// A grid combination rendered for logs and skip reports.
///
/// Sorted because `HashMap` iteration order varies between runs, and a log line that reorders
/// itself is not a log line you can diff two runs by.
#[cfg(feature = "execute")]
fn describe_params(params: &HashMap<String, Value>) -> String {
    if params.is_empty() {
        return "estimator defaults".to_string();
    }
    let mut entries: Vec<String> = params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    entries.sort();
    entries.join(", ")
}

/// Running state of one grid combination over the folds.
#[cfg(feature = "execute")]
struct Trial {
    params: HashMap<String, Value>,
    config: OrdinalConfig,
    fold_scores: Vec<f64>,
    elapsed_secs: f64,
    failure: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct OrdinalGridSearchNode {}

impl OrdinalGridSearchNode {
    pub fn new() -> Self {
        OrdinalGridSearchNode {}
    }
}

#[async_trait]
impl NodeLogic for OrdinalGridSearchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_tuning_ordinal_grid_search",
            "Ordinal Grid Search",
            "Exhaustively searches the hyperparameters of ONE ordinal model family with cross-validation, for a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Every combination in the Parameter Grid is scored on the SAME folds and ranked by an ordinal metric that knows how far a miss was. Use this rather than Grid Search, which resolves the target without its order and tunes against accuracy, scoring a five-level miss exactly like a one-level one. Model Type accepts the names Auto Ordinal reports as its best model, so the usual chain is Auto Ordinal to pick the family, then this node to tune it. Every family here is a gradient or a least-squares fit on the raw columns, so scale your features with the Fit Feature Scaler node first: unscaled columns change which hyperparameters win, not just how fast they converge.",
            "AI/ML/Tuning",
        );
        node.set_flowscript_name("ml", "ordinalGridSearch");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(3) // Refits every combination on every fold
                .set_governance(8)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts the sweep",
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
            "model_type",
            "Model Type",
            "Which ordinal family to tune. OrdinalLogistic is the threshold model, the widest family here: it takes a link, a loss and a margin, and covers proportional odds, ordered probit and support vector ordinal regression. OrdinalRidge is rank regression with learned cut points, closed-form and so by far the cheapest to sweep, but it has only a penalty to tune. OrdinalContinuationRatio models a sequential progression, `P(stop at k | reached k)`. OrdinalAdjacentCategory contrasts neighbouring levels instead of splitting the scale cumulatively. OrdinalNeural is a small network under a rank-consistent CORAL or CORN head, the only family here that is not linear in the features and the only one that can represent a level that is not monotone in them - and by a wide margin the most expensive to sweep, since every combination trains a whole network from scratch on every fold, so keep its grid small. Switching this after the Parameter Grid was seeded does NOT rewrite the grid - the run rejects parameters the new family does not consume rather than ignoring them silently.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    TUNABLE_ORDINAL_MODELS
                        .iter()
                        .map(|name| name.to_string())
                        .collect(),
                )
                .build(),
        )
        .set_default_value(Some(json!(KIND_LOGISTIC)));

        node.add_input_pin(
            "class_order",
            "Class Order",
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so the search fails rather than guessing one. The level set is resolved once here and handed to every fit, so a level that a fold happens to miss cannot renumber the ranks for that fold.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "cv_folds",
            "CV Folds",
            "How many folds the rows are split into. Every combination is scored on the SAME folds, so the comparison is paired rather than a race between different splits. More folds mean a less noisy score and proportionally more fitting, since the whole grid is refitted once per fold.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((2.0, 50.0)).build())
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "metric",
            "Metric",
            "What the sweep is ranked by. Quadratic Kappa is chance-corrected agreement that forgives a near miss and punishes a distant one four times as hard - the standard headline metric for ordered targets. Linear Kappa charges the same for every step along the scale, which is what you want when one level is one unit of loss. Mean Rank Error is the average number of levels a prediction is off by, and Macro Rank Error is the same averaged per true level so a rare level counts as much as the majority one - both are ERROR metrics, so the SMALLEST value wins and the `higher_is_better` output says so. Kendall Tau-b and Spearman ask only whether the rows come out in the right order and ignore calibration entirely, so a model whose levels are all shifted by one still scores perfectly.",
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
            "Seed for the fold shuffle, and for the weight initialization when Model Type is OrdinalNeural - the two sources of randomness in the sweep, tied to one value so the same seed reproduces the same folds, the same fits and therefore the same winner. Change it to check whether a narrow win survives a different split, which for the neural family also re-rolls the starting point of a non-convex fit. The winner is retrained from the same initialization it was scored at.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0.0, 4294967295.0)).build())
        .set_default_value(Some(json!(42)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the sweep completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Every combination that completed all folds with its mean and spread across the folds, plus the ones that were dropped and why. `higher_is_better` states which end of `mean_score` won.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalGridSearchResult>();

        node.add_output_pin(
            "best_model",
            "Best Model",
            "The winning combination retrained on the full dataset. Predictions come back as your original level labels.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "best_score",
            "Best Score",
            "Mean cross-validated score of the winner, in the units of the chosen metric. Meaningless without Higher Is Better: for the two error metrics this is the SMALLEST score in the sweep, not the largest.",
            VariableType::Float,
        );

        node.add_output_pin(
            "higher_is_better",
            "Higher Is Better",
            "Direction of the chosen metric: true for the agreement measures, false for MeanAbsoluteRankError and MacroMeanAbsoluteError, where a smaller score is the better model. Branch on this rather than assuming, otherwise a comparison downstream will rank the sweep upside down.",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "levels",
            "Levels",
            "The level order every configuration was trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when the results look upside down.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalLevels>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let model_type: String = context.evaluate_pin("model_type").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let cv_folds: i64 = context.evaluate_pin("cv_folds").await?;
        let metric_name: String = context.evaluate_pin("metric").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;
        let param_grid: Vec<ParameterSpec> = context.evaluate_pin("param_grid").await?;

        let metric = OrdinalMetric::parse(&metric_name)?;
        let family = OrdinalFamily::parse(&model_type)?;
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

        let start = Instant::now();

        // An empty grid means "use the defaults for this family", which keeps the node correct when
        // Model Type is switched after the pin was first seeded.
        let param_grid: Vec<ParameterSpec> = if param_grid.is_empty() {
            let defaults: Vec<ParameterSpec> =
                flow_like_types::json::from_value(default_param_grid(&model_type))
                    .unwrap_or_default();
            context.log_message(
                &format!("Parameter Grid is empty, using the default grid for {model_type}"),
                LogLevel::Info,
            );
            defaults
        } else {
            param_grid
        };

        // A spec with no values contributes nothing to the cartesian product and collapses it to
        // zero combinations, which would leave the best index pointing into an empty vector.
        if let Some(empty) = param_grid.iter().find(|spec| spec.values.is_empty()) {
            return Err(anyhow!(
                "Parameter `{}` in the Parameter Grid has no values to try. Give it at least one value or remove it.",
                empty.name
            ));
        }

        // Catches the grid left over from a previously selected Model Type. Every one of those
        // entries would be ignored by the builder, so the sweep would fit the same configuration N
        // times and report the identical scores as a tuning result.
        let accepted = family.known_params();
        let unknown: Vec<&str> = param_grid
            .iter()
            .map(|spec| spec.name.as_str())
            .filter(|name| !accepted.contains(name))
            .collect();
        if !unknown.is_empty() {
            // The pin is seeded once, at the model type selected when the node was placed, and is
            // deliberately never rewritten. Switching Model Type therefore lands here, so the
            // message has to carry the recovery rather than only the diagnosis.
            return Err(anyhow!(
                "{model_type} does not use these Parameter Grid entries: {}. It accepts: {}. The grid is seeded once when the node is placed and is not rewritten when Model Type changes, so this is what a leftover grid from a previous family looks like — clear the Parameter Grid to fall back to the defaults for {model_type}.",
                unknown.join(", "),
                accepted.join(", ")
            ));
        }

        // Derived from the actual product, never counted by hand: a hardcoded total drifts the
        // moment a parameter is added to or removed from the grid.
        let param_combinations = generate_param_combinations(&param_grid);
        let n_combinations = param_combinations.len();

        // Parsed before any data is loaded, so a malformed grid entry fails now instead of after
        // the first combination has already been fitted on every fold.
        let mut trials: Vec<Trial> = Vec::with_capacity(n_combinations);
        for params in param_combinations {
            let config = OrdinalConfig::parse(family, &params, seed)?;
            trials.push(Trial {
                params,
                config,
                fold_scores: Vec::with_capacity(cv_folds),
                elapsed_secs: 0.0,
                failure: None,
            });
        }

        let explicit_order: Vec<String> = class_order
            .split(',')
            .map(|level| level.trim())
            .filter(|level| !level.is_empty())
            .map(ToString::to_string)
            .collect();

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
        // Every family is either an Adam fit or a Cholesky solve, and both turn a single NaN into an
        // all-NaN model while only reporting that "the feature matrix" was non-finite.
        if let Some(((row, col), value)) = features
            .indexed_iter()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(anyhow!(
                "Training feature at row {row}, column {col} is {value}; ordinal fitting needs finite features. Clean or impute the column before searching."
            ));
        }
        // With fewer rows than folds, fold_size is 0: every fold but the last scores an empty
        // validation set and the last one trains on an empty split.
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
        // A wrong order tunes every combination confidently backwards and nothing in the results
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
                "Ordinal Grid Search on {model_type}: {n_samples} samples x {n_features} features, {n_levels} levels, {n_combinations} combinations, {cv_folds} folds, metric {}",
                metric.as_str()
            ),
            LogLevel::Info,
        );
        // The runtime jump is large enough that the size of the sweep is worth saying out loud
        // before it starts rather than after it has been running for an hour.
        if family == OrdinalFamily::Neural {
            context.log_message(
                &format!(
                    "Neural family selected: {n_combinations} combinations, each refitted once per fold across {cv_folds} folds, so {} network fits before the winner is retrained on the full data. A network is by far the most expensive estimator here - every other ordinal family is one gradient or least-squares fit over a single coefficient vector - so keep the grid small and add architectures one at a time. Its hidden layers are the only thing it adds: with none, CORAL is exactly the all-threshold model and CORN exactly Continuation Ratio, so if a linear family scores as well, prefer it.",
                    n_combinations.saturating_mul(cv_folds)
                ),
                LogLevel::Warn,
            );
        }

        let mut indices: Vec<usize> = (0..n_samples).collect();
        // Seeded on purpose: an unseeded shuffle makes the sweep irreproducible, and with several
        // combinations within noise of each other the winner would change between runs.
        let mut rng = StdRng::seed_from_u64(seed);
        indices.shuffle(&mut rng);
        let fold_size = n_samples / cv_folds;

        // Folds outermost: every combination sees the same split, and the split is materialised
        // once per fold instead of once per combination and fold.
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
                    .config
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
                        // One combination's fold failure must not end the sweep: an aggressive
                        // learning rate or a penalty of zero can diverge on one split while the
                        // rest of the grid is perfectly healthy.
                        let reason = format!("fold {} of {cv_folds}: {err}", fold + 1);
                        context.log_message(
                            &format!(
                                "Combination [{}] failed on {reason}. It is dropped from the results; the remaining combinations continue.",
                                describe_params(&trial.params)
                            ),
                            LogLevel::Warn,
                        );
                        trial.failure = Some(reason);
                    }
                }
            }
        }

        let skipped: Vec<OrdinalGridSearchSkip> = trials
            .iter()
            .filter_map(|trial| {
                trial.failure.as_ref().map(|reason| OrdinalGridSearchSkip {
                    params: trial.params.clone(),
                    reason: reason.clone(),
                })
            })
            .collect();
        let failure_summary = || {
            skipped
                .iter()
                .map(|skip| format!("[{}] ({})", describe_params(&skip.params), skip.reason))
                .collect::<Vec<_>>()
                .join("; ")
        };

        let Some(higher_is_better) = metric_direction else {
            return Err(anyhow!(
                "Every parameter combination failed to fit, so there is nothing to rank. Failures: {}",
                failure_summary()
            ));
        };

        let finished: Vec<(OrdinalConfig, HashMap<String, Value>, GridSearchEntry)> = trials
            .into_iter()
            .filter(|trial| trial.failure.is_none() && !trial.fold_scores.is_empty())
            .map(|trial| {
                let mean_score =
                    trial.fold_scores.iter().sum::<f64>() / trial.fold_scores.len() as f64;
                let variance = trial
                    .fold_scores
                    .iter()
                    .map(|score| (score - mean_score).powi(2))
                    .sum::<f64>()
                    / trial.fold_scores.len() as f64;
                let entry = GridSearchEntry {
                    params: trial.params.clone(),
                    mean_score,
                    std_score: variance.sqrt(),
                    fold_scores: trial.fold_scores,
                    train_time_secs: trial.elapsed_secs,
                };
                (trial.config, trial.params, entry)
            })
            .collect();

        if finished.is_empty() {
            return Err(anyhow!(
                "No parameter combination completed all {cv_folds} folds. Failures: {}",
                failure_summary()
            ));
        }

        // Compared through the metric's own direction rather than always taking the largest, which
        // is what keeps an error metric from crowning the worst combination in the grid.
        let mut best_index = 0usize;
        let mut best_score = finished[0].2.mean_score;
        for (index, (_, _, entry)) in finished.iter().enumerate().skip(1) {
            let improves = if higher_is_better {
                entry.mean_score > best_score
            } else {
                entry.mean_score < best_score
            };
            if improves {
                best_score = entry.mean_score;
                best_index = index;
            }
        }

        for (index, (_, params, entry)) in finished.iter().enumerate() {
            context.log_message(
                &format!(
                    "Combination {}/{}: [{}] {} = {:.4} +/- {:.4} in {:.2}s{}",
                    index + 1,
                    finished.len(),
                    describe_params(params),
                    metric.as_str(),
                    entry.mean_score,
                    entry.std_score,
                    entry.train_time_secs,
                    if index == best_index { "  <- best" } else { "" }
                ),
                LogLevel::Debug,
            );
        }

        let (best_config, best_params, _) = &finished[best_index];
        let best_config = best_config.clone();
        let best_params = best_params.clone();

        let dataset = DatasetBase::new(features, ranks);
        let final_model = best_config.fit_final(&dataset, n_levels, classes, &best_params)?;

        let result = OrdinalGridSearchResult {
            results: finished.into_iter().map(|(_, _, entry)| entry).collect(),
            skipped,
            best_index,
            best_params,
            best_score,
            model_type: final_model.kind().to_string(),
            metric: metric.as_str().to_string(),
            higher_is_better,
            total_time_secs: start.elapsed().as_secs_f64(),
            n_combinations,
            n_folds: cv_folds,
            n_levels,
            n_samples,
        };

        context.log_message(
            &format!(
                "Ordinal Grid Search complete: [{}] wins with {} = {:.4} ({} of {} combinations ranked, {} dropped) in {:.2}s",
                describe_params(&result.best_params),
                result.metric,
                result.best_score,
                result.results.len(),
                result.n_combinations,
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
            .set_pin_value("best_score", json!(best_score))
            .await?;
        context
            .set_pin_value("higher_is_better", json!(higher_is_better))
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
        let model_type: String = node
            .get_pin_by_name("model_type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let source_pin: String = node
            .get_pin_by_name("source")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        // Seeded once with the grid for whichever family was selected at the time, and deliberately
        // never re-seeded: rewriting a pin's default on every board parse would clobber a
        // hand-edited grid, and dropping and re-adding the pin would break any connection into it.
        // `run` substitutes the family's default grid when the pin is left empty and rejects
        // parameters the selected family does not consume, so switching Model Type is still safe.
        if node.get_pin_by_name("param_grid").is_none() {
            node.add_input_pin(
                "param_grid",
                "Parameter Grid",
                "Hyperparameters to search over, as a list of `{name, values}` entries; the sweep is their full cartesian product. Leave empty to use the default grid for the selected Model Type. Names must be ones the family consumes - OrdinalLogistic takes alpha, link, loss, margin, learning_rate and max_iterations; OrdinalRidge takes alpha; OrdinalContinuationRatio takes alpha, link and learning_rate; OrdinalAdjacentCategory takes alpha and learning_rate; OrdinalNeural takes alpha, head (`Coral` or `Corn`), activation (`Relu` or `Tanh`), hidden_layers, learning_rate and max_iterations - and anything else is rejected rather than ignored, because an ignored entry would score the same configuration over and over. hidden_layers is a STRING of comma-separated widths written exactly as the neural node's Hidden Layers pin takes them, e.g. `\"16\"` or `\"16, 8\"`, with `\"\"` for no hidden layer at all; every width must be at least 1. Keep the neural grid small - each of its combinations trains a network from scratch on every fold, and the default already crosses three architectures with two step sizes.",
                VariableType::Struct,
            )
            .set_schema::<Vec<ParameterSpec>>()
            .set_default_value(Some(default_param_grid(&model_type)));
        }

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

/// Full cartesian product of the grid.
///
/// An empty grid yields exactly one combination - the estimator's own defaults - rather than none,
/// which is what makes a family with nothing worth sweeping still produce a result.
#[cfg(feature = "execute")]
fn generate_param_combinations(grid: &[ParameterSpec]) -> Vec<HashMap<String, Value>> {
    let mut combinations = vec![HashMap::new()];

    for spec in grid {
        let mut expanded = Vec::with_capacity(combinations.len() * spec.values.len());
        for existing in &combinations {
            for value in &spec.values {
                let mut combination = existing.clone();
                combination.insert(spec.name.clone(), value.clone());
                expanded.push(combination);
            }
        }
        combinations = expanded;
    }

    combinations
}

/// Training dataset, validation features and the validation row indices of one CV fold.
#[cfg(feature = "execute")]
type FoldSplit = (
    DatasetBase<Array2<f64>, Array1<usize>>,
    Array2<f64>,
    Vec<usize>,
);

/// Materialises one cross-validation split from the shuffled row order.
#[cfg(feature = "execute")]
fn split_fold(
    features: &Array2<f64>,
    ranks: &Array1<usize>,
    indices: &[usize],
    fold: usize,
    fold_size: usize,
    cv_folds: usize,
) -> FoldSplit {
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
