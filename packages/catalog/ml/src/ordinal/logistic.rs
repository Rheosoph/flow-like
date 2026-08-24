//! Node for Fitting a **Proportional-Odds** (cumulative logit) ordinal model.
//!
//! The target's levels are ordered, and that ordering is the whole point: the model shares one
//! coefficient vector across every level and separates them with `K - 1` ordered cut points, so a
//! higher score can only move probability mass monotonically up the ordering.
//!
//! The Link and Loss pins widen this into the whole threshold-model family: the link picks the
//! latent distribution behind the cut points, while a threshold loss abandons the likelihood — and
//! with it every per-level probability — in exchange for dropping the proportional-odds assumption.
//!
//! Margin and Free Features widen it once more. A hinge margin on a threshold loss turns the fit
//! into support vector ordinal regression, and freeing a feature replaces its shared slope with one
//! slope per cut point — partial proportional odds, or the generalized model once every feature is
//! freed. Freed slopes are unconstrained, so the cumulative curves may cross; the Crossing Rate
//! output is the only thing that reveals it.

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
use flow_like_ordinal::{Link, Margin, OrdinalLogistic, OrdinalLoss};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// How far one freed feature's coefficient moves across the cut points
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FreeFeatureSpread {
    /// Position of the feature inside the training vector
    pub index: usize,
    /// Smallest coefficient the feature took at any cut point
    pub min: f64,
    /// Largest coefficient the feature took at any cut point
    pub max: f64,
    /// `max - min`. Near zero means one shared slope described the feature just as well, so freeing
    /// it only spent parameters.
    pub spread: f64,
}

/// The coefficient of every feature at every cut point of a fitted ordinal model
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalEffectiveCoefficients {
    /// One row per cut point, lowest cut first, each row holding one coefficient per feature in
    /// column order. A shared feature repeats the same value down every row; a freed one varies.
    pub per_threshold: Vec<Vec<f64>>,
    /// The fitted cut points on the latent scale, strictly increasing, aligned row for row with
    /// `per_threshold`
    pub thresholds: Vec<f64>,
    /// Feature indices that were freed, sorted and de-duplicated as the fit received them
    pub free_features: Vec<usize>,
    /// One entry per freed feature, widest spread first
    pub free_feature_spread: Vec<FreeFeatureSpread>,
    /// Number of feature columns
    pub n_features: usize,
    /// Number of cut points, i.e. one less than the number of levels
    pub n_thresholds: usize,
}

#[crate::register_node]
#[derive(Default)]
pub struct FitOrdinalLogisticNode {}

impl FitOrdinalLogisticNode {
    pub fn new() -> Self {
        FitOrdinalLogisticNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOrdinalLogisticNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_ordinal_logistic",
            "Train Ordinal Model (Proportional Odds)",
            "Fit/Train a proportional-odds model on a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Use this instead of a classifier, which treats the levels as unrelated names and so counts predicting `low` for `high` as no worse than predicting `medium`. Use it instead of a regressor, which treats the levels as real numbers and so invents distances the levels do not carry (`high` is not exactly twice `medium`). The model learns one coefficient vector plus ordered cut points, which keeps predictions monotone in the score and, under the default loss, yields calibrated per-level probabilities. Link Function, Loss and Margin widen it to the whole threshold-model family, up to support vector ordinal regression, while Free Features relaxes the shared coefficient into one slope per cut point. Scale your features first with the Fit Feature Scaler node: this is a gradient fit, and unscaled columns make it converge slowly or not at all.",
            "AI/ML/Ordinal",
        );
        node.set_flowscript_name("ml", "fitOrdinalLogistic");
        node.set_version(3);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(8) // One coefficient per feature, sign readable as direction along the ordering
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins ordinal model training",
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
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "link",
            "Link Function",
            "The CDF sitting behind the cut points, i.e. which latent distribution you assume produced the levels. Logit gives the proportional-odds model and coefficients that read as log odds ratios. Probit assumes a normally distributed latent variable and is the convention in econometrics and the social sciences. CLogLog is asymmetric — it leaves the bottom level quickly and approaches the top one slowly — which is the right shape for `time until something escalates` targets. Cauchit is heavy-tailed, so extreme rows pull the fit far less than they do under Logit or Probit. Applies to the CumulativeLink loss only: the two threshold losses use a logistic margin and ignore this.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Logit".to_string(),
                    "Probit".to_string(),
                    "CLogLog".to_string(),
                    "Cauchit".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Logit")));

        node.add_input_pin(
            "loss",
            "Loss",
            "What the optimizer actually minimizes. CumulativeLink maximizes the likelihood of each level and is the ONLY choice that carries a probability model — the confidence value on the Predict node comes from it. AllThreshold penalizes every cut point that falls on the wrong side of the observation, ImmediateThreshold only the two bracketing it; both drop the proportional-odds assumption and are often more robust when it fails, but they fit cut-point placement rather than a likelihood, so the resulting model yields NO per-level probabilities and Predict returns no confidence.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "CumulativeLink".to_string(),
                    "AllThreshold".to_string(),
                    "ImmediateThreshold".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("CumulativeLink")));

        node.add_input_pin(
            "margin",
            "Margin",
            "Shape of the penalty a cut point pays for sitting on the wrong side of an observation. Hinge charges nothing once the cut point clears the margin, so only the observations NEAR a cut point influence the fit at all: Hinge together with the AllThreshold loss IS support vector ordinal regression (Chu & Keerthi's implicit-constraint SVOR), and with ImmediateThreshold it is the explicit-constraint variant. SquaredHinge is the differentiable version of that kink — smoother gradients, but distant violations are punished quadratically, so single outliers drag the cut points. Logistic is smooth everywhere and charges even well-placed cut points a little. IGNORED by the default CumulativeLink loss, which maximizes a likelihood and has no margin.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Logistic".to_string(),
                    "Hinge".to_string(),
                    "SquaredHinge".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Logistic")));

        node.add_input_pin(
            "free_features",
            "Free Features",
            "Comma-separated feature INDICES (0-based, e.g. `0, 3`) that get their own coefficient at EVERY cut point instead of one shared across all of them — the partial proportional-odds model. Empty is the standard model, where a single slope describes every cut point; that is an assumption. Free a feature when you suspect it violates it, then check the Effective Coefficients output: a feature whose per-cut slopes barely differ gained nothing by being freed. Freeing only the ones that do differ keeps every other feature parsimonious. Listing every index gives the fully generalized ordinal model. The price shows up on Crossing Rate: unconstrained per-cut slopes let the cumulative curves cross, which is no longer a valid probability model.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "alpha",
            "Alpha (L2 Penalty)",
            "Strength of the L2 penalty on the coefficients; the cut points are never penalized. 0 fits unpenalized. Raise it when the fit diverges or the coefficients blow up.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "max_iterations",
            "Max Iterations",
            "Iteration cap for the Adam optimizer. Training stops here even if the objective is still moving, which is reported on the Converged pin.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 1_000_000.0)).build())
        .set_default_value(Some(json!(500)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Relative change in the objective below which training stops. Smaller values fit tighter but need more iterations; 0 always runs the full iteration budget.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(1e-7)));

        node.add_input_pin(
            "learning_rate",
            "Learning Rate",
            "Adam step size. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((1e-6, 10.0)).build())
        .set_default_value(Some(json!(0.1)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained proportional-odds model. Predictions come back as your original level labels.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "levels",
            "Levels",
            "The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalLevels>();

        node.add_output_pin(
            "converged",
            "Converged",
            "False when the optimizer hit Max Iterations before the objective settled. The model is still usable but under-fitted.",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "crossing_rate",
            "Crossing Rate",
            "Share of training rows (0.0 to 1.0) whose cumulative curves crossed, i.e. where the fit put P(y <= k) ABOVE P(y <= k+1) and so implied a negative probability for a level. Always 0.0 without Free Features, because a shared slope cannot cross. Anything above 0 means the generalized fit is no longer a clean probability model: prediction clamps and renormalizes so nothing downstream sees a negative number, but the per-level probabilities stop being trustworthy — free fewer features, or go back to the shared model.",
            VariableType::Float,
        );

        node.add_output_pin(
            "effective_coefficients",
            "Effective Coefficients",
            "The coefficient of every feature at every cut point, one row per cut point from lowest to highest, next to the cut points themselves. Shared features repeat the same value down every row; freed ones vary, and the reported spread (largest minus smallest over the cut points) is how you tell whether freeing a feature bought anything — a spread near zero means one shared slope fitted it just as well and the extra parameters were wasted.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalEffectiveCoefficients>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let alpha: f64 = context.evaluate_pin("alpha").await?;
        let max_iterations: i64 = context.evaluate_pin("max_iterations").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;
        let learning_rate: f64 = context.evaluate_pin("learning_rate").await?;
        // Boards placed before these pins existed fall back to the crate defaults, which is exactly
        // what the node used to pass.
        let link: String = context
            .evaluate_pin("link")
            .await
            .unwrap_or_else(|_| "Logit".to_string());
        let loss: String = context
            .evaluate_pin("loss")
            .await
            .unwrap_or_else(|_| "CumulativeLink".to_string());
        let margin: String = context
            .evaluate_pin("margin")
            .await
            .unwrap_or_else(|_| "Logistic".to_string());
        let free_features_raw: String = context
            .evaluate_pin("free_features")
            .await
            .unwrap_or_default();

        let link = match link.as_str() {
            "Logit" => Link::Logit,
            "Probit" => Link::Probit,
            "CLogLog" => Link::CLogLog,
            "Cauchit" => Link::Cauchit,
            other => {
                return Err(anyhow!(
                    "Unknown link function `{other}`, expected `Logit`, `Probit`, `CLogLog` or `Cauchit`"
                ));
            }
        };
        let loss = match loss.as_str() {
            "CumulativeLink" => OrdinalLoss::CumulativeLink,
            "AllThreshold" => OrdinalLoss::AllThreshold,
            "ImmediateThreshold" => OrdinalLoss::ImmediateThreshold,
            other => {
                return Err(anyhow!(
                    "Unknown loss `{other}`, expected `CumulativeLink`, `AllThreshold` or `ImmediateThreshold`"
                ));
            }
        };
        let margin = match margin.as_str() {
            "Logistic" => Margin::Logistic,
            "Hinge" => Margin::Hinge,
            "SquaredHinge" => Margin::SquaredHinge,
            other => {
                return Err(anyhow!(
                    "Unknown margin `{other}`, expected `Logistic`, `Hinge` or `SquaredHinge`"
                ));
            }
        };

        let mut free_features: Vec<usize> = Vec::new();
        for token in free_features_raw.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let index = token.parse::<usize>().map_err(|_| {
                anyhow!(
                    "`Free Features` takes comma-separated 0-based feature indices, but `{token}` is not a non-negative integer. Leave the pin empty for the standard proportional-odds model."
                )
            })?;
            free_features.push(index);
        }
        free_features.sort_unstable();
        free_features.dedup();

        if !alpha.is_finite() || alpha < 0.0 {
            return Err(anyhow!(
                "`Alpha (L2 Penalty)` must be a finite value >= 0, got {alpha}"
            ));
        }
        if !(1..=u32::MAX as i64).contains(&max_iterations) {
            return Err(anyhow!(
                "`Max Iterations` must be between 1 and {}, got {max_iterations}",
                u32::MAX
            ));
        }
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(anyhow!(
                "`Tolerance` must be a finite value >= 0, got {tolerance}"
            ));
        }
        if !learning_rate.is_finite() || learning_rate <= 0.0 {
            return Err(anyhow!(
                "`Learning Rate` must be a finite value > 0, got {learning_rate}"
            ));
        }

        let explicit_order: Vec<String> = class_order
            .split(',')
            .map(|level| level.trim())
            .filter(|level| !level.is_empty())
            .map(ToString::to_string)
            .collect();

        let t0 = std::time::Instant::now();
        let (train_array, ranks, classes, levels) = match source.as_str() {
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
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );
                if records.is_empty() {
                    return Err(anyhow!(
                        "No training records in the database; ordinal fitting needs at least one row"
                    ));
                }

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (ranks, classes, levels) = values_to_array1_ordinal(
                    &records,
                    &targets_col,
                    if explicit_order.is_empty() {
                        None
                    } else {
                        Some(explicit_order.as_slice())
                    },
                )?;
                (train_array, ranks, classes, levels)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        let (n_samples, n_features) = train_array.dim();
        if n_features == 0 {
            return Err(anyhow!(
                "Training records have 0 features, expected at least one value per row"
            ));
        }
        if let Some(out_of_range) = free_features.iter().find(|index| **index >= n_features) {
            return Err(anyhow!(
                "`Free Features` names column {out_of_range}, but the training records have {n_features} columns, so the valid indices are 0 to {}",
                n_features - 1
            ));
        }
        // Adam turns a single NaN into an all-NaN parameter vector, and the crate would only report
        // that "the feature matrix" was non-finite, so the offending cell is resolved here.
        if let Some(((row, col), value)) = train_array
            .indexed_iter()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(anyhow!(
                "Training feature at row {row}, column {col} is {value}; ordinal fitting needs finite features. Clean or impute the column before training."
            ));
        }

        let n_classes = levels.labels.len();
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
        // Training on a wrong order fails silently — the model just learns the wrong direction — so
        // the resolved order has to be visible in the run log, not only on the output pin.
        context.log_message(
            &format!(
                "Ordinal level order ({ordering_source}): {}",
                levels.labels.join(" < ")
            ),
            LogLevel::Info,
        );

        // A margin picked under the likelihood loss is a silent no-op, and a hinge margin under a
        // threshold loss quietly changes which model this is. Both are worth saying out loud.
        if margin != Margin::Logistic && !loss.is_threshold_loss() {
            context.log_message(
                &format!(
                    "The {margin:?} margin is ignored: {loss:?} maximizes a likelihood and has no margin. Switch Loss to AllThreshold or ImmediateThreshold for the margin to take effect."
                ),
                LogLevel::Warn,
            );
        }
        if margin == Margin::Hinge && loss.is_threshold_loss() {
            context.log_message(
                &format!(
                    "Hinge margin with the {loss:?} loss: this is support vector ordinal regression, so only observations within the margin of a cut point influence the fit."
                ),
                LogLevel::Info,
            );
        }
        if !free_features.is_empty() {
            let freed = free_features
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            context.log_message(
                &format!(
                    "Partial proportional odds: features [{freed}] get their own coefficient at each of the {} cut points, the remaining {} share one. Check Crossing Rate afterwards.",
                    n_classes - 1,
                    n_features - free_features.len()
                ),
                LogLevel::Info,
            );
        }

        let t0 = std::time::Instant::now();
        // `n_levels` is declared rather than inferred: an explicit Class Order may name levels the
        // training sample never reached, and the thresholds must keep a slot for them.
        let dataset = DatasetBase::new(train_array, ranks);
        let fitted = OrdinalLogistic::params()
            .link(link)
            .loss(loss)
            .margin(margin)
            .free_features(&free_features)
            .alpha(alpha)
            .max_iterations(max_iterations as usize)
            .tolerance(tolerance)
            .learning_rate(learning_rate)
            .n_levels(n_classes)
            .fit(&dataset)
            .map_err(|err| {
                anyhow!(
                    "Ordinal fit ({link:?} link, {loss:?} loss, {margin:?} margin) failed: {err}"
                )
            })?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        let converged = fitted.converged();
        context.log_message(
            &format!(
                "Ordinal model fit on {n_samples} samples x {n_features} features, {n_classes} levels, {link:?} link, {loss:?} loss, {} iterations",
                fitted.iterations()
            ),
            LogLevel::Debug,
        );
        // A threshold loss carries no probability model, so the Predict node downstream will report
        // an empty confidence. Saying so here beats leaving the user to guess why it vanished.
        if loss.is_threshold_loss() {
            context.log_message(
                &format!(
                    "{loss:?} fits cut-point placement rather than a likelihood, so this model has no per-level probabilities: the Predict node will return no confidence value. Refit with the CumulativeLink loss if you need them."
                ),
                LogLevel::Info,
            );
        }
        if !converged {
            context.log_message(
                &format!(
                    "Training stopped at the cap of {} iterations without converging. The model is under-fitted: raise Max Iterations, raise Learning Rate, or scale the features with the Fit Feature Scaler node.",
                    fitted.iterations()
                ),
                LogLevel::Warn,
            );
        }

        let crossing_rate = fitted.crossing_rate();
        // Nothing downstream fails on a crossing fit — prediction clamps — so an unread rate is a
        // degenerate model that looks healthy.
        if crossing_rate > 0.0 {
            context.log_message(
                &format!(
                    "Cumulative curves crossed on {:.1}% of the training rows: the per-cut slopes of the {} freed feature(s) put P(y <= k) above P(y <= k+1), which implies a negative probability for a level. Prediction clamps and renormalizes, so nothing downstream breaks, but this is no longer a clean probability model. Free fewer features, or drop back to the shared proportional-odds fit.",
                    crossing_rate * 100.0,
                    free_features.len()
                ),
                LogLevel::Warn,
            );
        }

        let per_threshold: Vec<Vec<f64>> = fitted
            .effective_coefficients()
            .outer_iter()
            .map(|row| row.to_vec())
            .collect();
        let mut free_feature_spread: Vec<FreeFeatureSpread> = fitted
            .free_features()
            .iter()
            .map(|index| {
                let (min, max) = per_threshold
                    .iter()
                    .filter_map(|row| row.get(*index))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
                        (min.min(*value), max.max(*value))
                    });
                FreeFeatureSpread {
                    index: *index,
                    min,
                    max,
                    spread: max - min,
                }
            })
            .collect();
        free_feature_spread.sort_by(|a, b| b.spread.total_cmp(&a.spread));
        let effective_coefficients = OrdinalEffectiveCoefficients {
            thresholds: fitted.thresholds().to_vec(),
            free_features: fitted.free_features().to_vec(),
            free_feature_spread,
            n_features,
            n_thresholds: per_threshold.len(),
            per_threshold,
        };

        let model = MLModel::OrdinalLogistic(ModelWithMeta {
            model: fitted,
            classes: Some(classes),
        });
        let node_model = NodeMLModel::new(context, model).await;

        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("levels", json!(levels)).await?;
        context.set_pin_value("converged", json!(converged)).await?;
        context
            .set_pin_value("crossing_rate", json!(crossing_rate))
            .await?;
        context
            .set_pin_value("effective_coefficients", json!(effective_coefficients))
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
