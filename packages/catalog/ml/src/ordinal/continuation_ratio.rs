//! Node for Fitting a **Continuation-Ratio** ordinal model.
//!
//! Every other ordinal node in this catalog asks one question about the whole scale — how far up is
//! this row. This one asks a different question `K - 1` times: given that the row REACHED level `k`,
//! did it stop there? That makes the model a description of a sequential progression that can halt
//! at each step, rather than a set of cut points on a latent scale.
//!
//! Sub-model `k` is fitted only on the rows with `y >= k`, so the training subsets shrink as the
//! level rises and the top sub-models rest on the least evidence. Subset Sizes exists to make that
//! visible instead of leaving it as a property of the family nobody reads about.

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
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_ordinal::{ContinuationRatio, Link};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Rows per estimated parameter below which a sub-model is called out as thin.
///
/// Each sub-model estimates `n_features + 1` numbers from its own conditioning subset, and the long
/// standing rule of thumb for a binary regression fit is around ten observations per estimated
/// parameter. Below that the coefficients are dominated by which rows happened to be sampled, and on
/// this family that always bites the top levels first.
#[cfg(feature = "execute")]
const MIN_ROWS_PER_PARAMETER: usize = 10;

#[crate::register_node]
#[derive(Default)]
pub struct FitOrdinalContinuationRatioNode {}

impl FitOrdinalContinuationRatioNode {
    pub fn new() -> Self {
        FitOrdinalContinuationRatioNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOrdinalContinuationRatioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_ordinal_continuation_ratio",
            "Train Ordinal Model (Continuation Ratio)",
            "Fit/Train a continuation-ratio model on an ORDERED target that is really a process that can halt. It fits K-1 sub-models, where sub-model k answers `given this row reached level k, did it STOP there?`, so the model describes a progression through the levels instead of placing cut points on a latent scale. Reach for it when the levels are genuinely sequential and each one had to be passed to get to the next: escalation tiers, disease stages, how far a signup funnel got, how far an incident escalated before it was contained. Each sub-model carries its own coefficient vector, so nothing assumes proportional odds, and the per-level probabilities are exact by the chain rule rather than differences of two fits. The cost is strictness: because each sub-model is conditioned on having reached its level, EVERY level must occur in the training data, middle ones included. Scale your features first with the Fit Feature Scaler node: these are gradient fits, and unscaled columns make them converge slowly or not at all.",
            "AI/ML/Ordinal",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(6) // K-1 separate Adam fits instead of one
                .set_governance(7) // Readable coefficients, but one vector per level rather than one shared
                .set_reliability(6) // The high levels are fitted on the fewest rows, by construction
                .set_cost(6)
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
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Unlike the other ordinal nodes, a level you list here that never occurs in the data is rejected instead of merely left unpredicted: its sub-model would have no rows to separate.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "link",
            "Link Function",
            "The CDF each conditional stopping probability is read through. CLogLog is the standout pairing here: with it this model IS the discrete-time proportional-hazards (grouped survival) model, each sub-model's output is the hazard of stopping at that step, and a shared feature effect multiplies every hazard by the same factor — so for `how long / how far until something stopped` targets, pick CLogLog and read the fit as a survival model. Logit gives conditional log-odds, the classical continuation-ratio logit, and is the safe default. Probit assumes a normal latent variable per step. Cauchit is heavy-tailed, so extreme rows pull each sub-model far less.",
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
            "alpha",
            "Alpha (L2 Penalty)",
            "Strength of the L2 penalty on each sub-model's coefficients; the intercepts are never penalized. Because the penalty is a fixed amount added to a summed log-likelihood, one value shrinks the high levels harder than the low ones — which is what you want, since those are the sub-models fitted on the fewest rows. Raise it when Subset Sizes shows a thin top end.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "max_iterations",
            "Max Iterations",
            "Iteration cap for the Adam optimizer, applied to EACH sub-model separately. A single sub-model stopping here makes Converged false.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 1_000_000.0)).build())
        .set_default_value(Some(json!(500)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Relative change in a sub-model's objective below which its fit stops. The test is relative, so it means the same thing on the large bottom subset and the small top one. 0 always runs the full iteration budget.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(1e-7)));

        node.add_input_pin(
            "learning_rate",
            "Learning Rate",
            "Adam step size, shared by every sub-model. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations.",
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
            "Thread-safe handle to the trained continuation-ratio model. Predictions come back as your original level labels, and the per-level probabilities behind them sum to exactly 1 because the chain rule telescopes.",
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
            "subset_sizes",
            "Subset Sizes",
            "How many training rows each sub-model actually saw, lowest level first: entry k counts the rows that reached level k. It only ever decreases, so the LAST entry is the evidence behind your top level — the honest measure of how much to trust the high end of the fit. A small tail there means the top coefficients are noise, not a subtle effect.",
            VariableType::Integer,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "converged",
            "Converged",
            "True only when EVERY sub-model's objective settled before Max Iterations. One stubborn sub-model — usually the top one, fitted on the fewest rows — makes it false; the run log names which levels stalled.",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let link: String = context.evaluate_pin("link").await?;
        let alpha: f64 = context.evaluate_pin("alpha").await?;
        let max_iterations: i64 = context.evaluate_pin("max_iterations").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;
        let learning_rate: f64 = context.evaluate_pin("learning_rate").await?;

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

        // The crate rejects an unfittable conditioning subset by rank number; catching it here first
        // lets the message name the level the user actually wrote.
        let mut counts = vec![0usize; n_classes];
        for rank in ranks.iter() {
            counts[*rank] += 1;
        }
        if let Some(rank) = counts.iter().position(|count| *count == 0) {
            let label = levels
                .labels
                .get(rank)
                .cloned()
                .unwrap_or_else(|| rank.to_string());
            return Err(anyhow!(
                "Level `{label}` (rank {rank} of {n_classes}) never occurs in the training data. The continuation-ratio family fits one sub-model per level, conditioned on having reached it, so an absent level leaves its sub-model with nothing to separate — every declared level must occur, MIDDLE ones included. This is stricter than the proportional-odds and Frank & Hall families, which tolerate a gap. Drop `{label}` from Class Order, merge it into a neighbouring level, or widen the training set."
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

        let t0 = std::time::Instant::now();
        // `n_levels` is declared rather than inferred so a Class Order that names a level the sample
        // never reaches is rejected instead of being quietly dropped off the top of the scale.
        let dataset = DatasetBase::new(train_array, ranks);
        let fitted = ContinuationRatio::params()
            .link(link)
            .alpha(alpha)
            .max_iterations(max_iterations as usize)
            .tolerance(tolerance)
            .learning_rate(learning_rate)
            .n_levels(n_classes)
            .fit(&dataset)
            .map_err(|err| {
                anyhow!(
                    "Continuation-ratio fit ({link:?} link) failed: {err}. Each sub-model is fitted only on the rows that reached its level, so a level that never occurs, one nothing ever continues past, or one nothing ever stops at cannot be fitted at all — this family is stricter than the others about the target's coverage."
                )
            })?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        let subset_sizes = fitted.subset_sizes().to_vec();
        let converged = fitted.converged();

        // The shrinking subsets are the defining property of this family and the reason the top of
        // the scale is the least trustworthy part of the fit, so the ladder goes in the log too.
        let ladder = subset_sizes
            .iter()
            .enumerate()
            .map(|(level, size)| {
                let label = levels
                    .labels
                    .get(level)
                    .cloned()
                    .unwrap_or_else(|| level.to_string());
                format!("`{label}`: {size} rows")
            })
            .collect::<Vec<_>>()
            .join(", ");
        context.log_message(
            &format!("Conditioning subsets (rows that reached each level): {ladder}"),
            LogLevel::Info,
        );
        context.log_message(
            &format!(
                "Continuation-ratio model fit on {n_samples} samples x {n_features} features, {n_classes} levels, {link:?} link, iterations per sub-model {:?}",
                fitted.iterations()
            ),
            LogLevel::Debug,
        );

        let thin_threshold = MIN_ROWS_PER_PARAMETER * (n_features + 1);
        if let Some((level, size)) = subset_sizes
            .iter()
            .enumerate()
            .min_by_key(|(_, size)| **size)
            && *size < thin_threshold
        {
            let label = levels
                .labels
                .get(level)
                .cloned()
                .unwrap_or_else(|| level.to_string());
            context.log_message(
                &format!(
                    "The sub-model for level `{label}` was fitted on only {size} rows while estimating {} parameters ({n_features} features plus an intercept). Below roughly {MIN_ROWS_PER_PARAMETER} rows per parameter its coefficients mostly reflect which rows happened to be sampled. Merge the thin levels into their neighbour, raise Alpha to shrink them, or gather more rows at the top of the scale.",
                    n_features + 1
                ),
                LogLevel::Warn,
            );
        }

        if !converged {
            let stalled = fitted
                .converged_per_level()
                .iter()
                .enumerate()
                .filter(|(_, settled)| !**settled)
                .map(|(level, _)| {
                    levels
                        .labels
                        .get(level)
                        .cloned()
                        .unwrap_or_else(|| level.to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            context.log_message(
                &format!(
                    "Sub-models for level(s) [{stalled}] hit the cap of {max_iterations} iterations without converging. Those levels are under-fitted: raise Max Iterations, raise Learning Rate, raise Alpha if their conditioning subsets are thin, or scale the features with the Fit Feature Scaler node."
                ),
                LogLevel::Warn,
            );
        }

        let model = MLModel::OrdinalContinuationRatio(ModelWithMeta {
            model: fitted,
            classes: Some(classes),
        });
        let node_model = NodeMLModel::new(context, model).await;

        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("levels", json!(levels)).await?;
        context
            .set_pin_value("subset_sizes", json!(subset_sizes))
            .await?;
        context.set_pin_value("converged", json!(converged)).await?;
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
