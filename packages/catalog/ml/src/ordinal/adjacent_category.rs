//! Node for Fitting an **Adjacent-Category** ordinal model.
//!
//! The model contrasts each level with the one immediately below it, so a coefficient here is the
//! log odds of stepping up exactly ONE level. That is the reading most people already assume an
//! ordinal coefficient carries, and the cumulative (proportional-odds) node is the one that does
//! not provide it: there the same-looking number contrasts everything at or below a cut point
//! against everything above it.
//!
//! No link pin, deliberately: the family is defined by a log-ratio between neighbouring levels, and
//! there is no latent CDF to pick a shape for.

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
use flow_like_ordinal::AdjacentCategory;
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

/// Fitted parameters of an adjacent-category model, every one of them a PER-STEP quantity.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdjacentCategoryCoefficients {
    /// One coefficient per feature, shared by every adjacent pair of levels. `exp(value)` is the
    /// factor one unit of that feature applies to the odds of landing on level `k + 1` rather than
    /// on level `k`, and it is the same factor at every `k`. It is NOT the cumulative odds ratio a
    /// proportional-odds fit reports: do not compare the two numbers directly.
    pub coefficients: Vec<f64>,
    /// Number of input features.
    pub n_features: usize,
    /// The `n_levels - 1` fitted level contrasts, lowest pair first: entry `k` is the log odds of
    /// level `k + 1` against level `k` for a sample whose score is zero. These are free intercepts,
    /// one per adjacent pair — NOT ordered cut points. They may DECREASE, which only means that
    /// level is rarer than the one below it, and forcing an order on them would be wrong here.
    pub level_contrasts: Vec<f64>,
    /// Number of ordered levels the model was fitted on.
    pub n_levels: usize,
    /// `(n_levels - 1) * coefficient`: the total log-odds effect of one unit of each feature across
    /// the whole ordering, because a shared coefficient accumulates once per step. Reported next to
    /// the per-step value because this is the magnitude a cumulative coefficient is usually quoted
    /// at, and swapping the two silently understates the effect by that factor.
    pub bottom_to_top_effect: Vec<f64>,
}

#[crate::register_node]
#[derive(Default)]
pub struct FitOrdinalAdjacentCategoryNode {}

impl FitOrdinalAdjacentCategoryNode {
    pub fn new() -> Self {
        FitOrdinalAdjacentCategoryNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOrdinalAdjacentCategoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_ordinal_adjacent_category",
            "Train Ordinal Model (Adjacent Category)",
            "Fit/Train an ordinal model that compares each level with the one directly below it: `log( P(level k+1) / P(level k) ) = contrast_k + x . beta`. Its coefficients answer `what does one more unit of this feature do to my rating?` - `exp(coefficient)` is the factor on the odds of scoring one level higher rather than staying put, the same factor at every step. That is NOT what Train Ordinal Model (Proportional Odds) reports: a cumulative coefficient is the log odds ratio of everything AT OR BELOW a cut point against everything above it, pooling levels instead of comparing two neighbours. The same fitted number therefore means different things in the two families, and since one shared coefficient applies once per step here, the bottom-to-top effect is (levels - 1) times the per-step effect. Pick this for ratings, severity grades and Likert answers, where the question really is about one step; pick proportional odds when the question is about crossing a threshold (`does this case escalate past level 2?`). Fitted by penalized maximum likelihood over all levels jointly, so per-level probabilities are calibrated and the Predict node returns a confidence. Scale your features first with the Fit Feature Scaler node: this is a gradient fit, and unscaled columns make it converge slowly or not at all.",
            "AI/ML/Ordinal",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(9) // One coefficient per feature, and its per-step reading is the one people already have in mind
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins adjacent-category training",
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
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Levels listed here but never seen in training still keep their slot in the ordering, so the contrasts stay comparable across runs.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "alpha",
            "Alpha (L2 Penalty)",
            "Strength of the L2 penalty on the shared coefficients. The level contrasts are never penalized: shrinking those would pull neighbouring levels toward equal frequency, which asserts something about your data rather than limiting model complexity. 0 fits unpenalized. Raise it when the fit diverges or the coefficients blow up.",
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
            "Adam step size. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations. Level scores here carry a factor of the level index, so a badly scaled step travels further than it would in a cumulative fit.",
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
            "Thread-safe handle to the trained adjacent-category model. Predictions come back as your original level labels, and because the fit maximizes a likelihood the Predict node also returns a per-level confidence.",
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
            "coefficients",
            "Coefficients",
            "The shared per-feature coefficients together with the level contrasts, both of them PER-STEP quantities: `exp(coefficient)` multiplies the odds of landing one level higher rather than on the current one, which is a single step and not the cumulative `above this cut` odds ratio a proportional-odds model prints. The struct also carries `bottom_to_top_effect`, the same coefficient times (levels - 1), which is the magnitude to quote when someone asks about the full range. The contrasts are the same log odds at a zero score, one per adjacent pair; unlike cumulative cut points they are free intercepts and may DECREASE.",
            VariableType::Struct,
        )
        .set_schema::<AdjacentCategoryCoefficients>();

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
        // `n_levels` is declared rather than inferred: an explicit Class Order may name levels the
        // training sample never reached, and every adjacent pair must keep its contrast.
        let dataset = DatasetBase::new(train_array, ranks);
        let fitted = AdjacentCategory::params()
            .alpha(alpha)
            .max_iterations(max_iterations as usize)
            .tolerance(tolerance)
            .learning_rate(learning_rate)
            .n_levels(n_classes)
            .fit(&dataset)
            .map_err(|err| anyhow!("Adjacent-category fit failed: {err}"))?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        let converged = fitted.converged();
        let coefficients: Vec<f64> = fitted.coefficients().to_vec();
        let level_contrasts: Vec<f64> = fitted.thresholds().to_vec();
        let steps = n_classes.saturating_sub(1) as f64;

        context.log_message(
            &format!(
                "Adjacent-category model fit on {n_samples} samples x {n_features} features, {n_classes} levels, {} iterations, level contrasts {level_contrasts:?}",
                fitted.iterations()
            ),
            LogLevel::Debug,
        );
        // A cumulative model would be broken if its cut points went backwards, so anyone arriving
        // from that node reads a dip here as a bug. It is not one, and the log is the only place
        // that distinction can be made before someone acts on it.
        if level_contrasts.windows(2).any(|pair| pair[1] < pair[0]) {
            context.log_message(
                "Some level contrasts decrease. That is legitimate for this family: they are free intercepts, one per adjacent pair, not the ordered cut points of a cumulative model. A dip only says that level is rarer than the one below it.",
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

        let coefficient_result = AdjacentCategoryCoefficients {
            n_features: coefficients.len(),
            bottom_to_top_effect: coefficients.iter().map(|value| value * steps).collect(),
            coefficients,
            level_contrasts,
            n_levels: n_classes,
        };

        let model = MLModel::OrdinalAdjacentCategory(ModelWithMeta {
            model: fitted,
            classes: Some(classes),
        });
        let node_model = NodeMLModel::new(context, model).await;

        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("levels", json!(levels)).await?;
        context.set_pin_value("converged", json!(converged)).await?;
        context
            .set_pin_value("coefficients", json!(coefficient_result))
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
