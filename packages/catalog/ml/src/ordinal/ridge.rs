//! Node for Fitting an **Ordinal Ridge** model.
//!
//! Regresses the level *rank* on the features with an L2 penalty, then cuts the resulting score at
//! thresholds learned from the training distribution. The closed-form counterpart to the
//! proportional-odds model in [`flow_like_ordinal::logistic`].

use crate::ml::{LinearCoefficients, NodeMLModel, OrdinalLevels};
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
use flow_like_ordinal::OrdinalRidge;
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
use ndarray::Array2;
#[cfg(feature = "execute")]
use std::collections::{BTreeSet, HashSet};

/// Feature width beyond which the solve is worth warning about: the fit materializes a
/// `p x p` Gram matrix and Cholesky-factorizes it, so cost grows as `p^2` in memory and `p^3` in
/// time. Embedding columns are commonly 384-1536 wide, which is already tens of megabytes.
#[cfg(feature = "execute")]
const WIDE_FEATURE_WARNING: usize = 256;

/// Position of the first non-finite feature, if any. The fit rejects the whole matrix with a single
/// message, which is useless for finding the offending cell.
#[cfg(feature = "execute")]
fn first_non_finite(records: &Array2<f64>) -> Option<(usize, usize)> {
    records
        .indexed_iter()
        .find(|(_, value)| !value.is_finite())
        .map(|((row, col), _)| (row, col))
}

/// Splits the user supplied `Class Order` into levels, lowest first.
#[cfg(feature = "execute")]
fn parse_class_order(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|level| !level.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[crate::register_node]
#[derive(Default)]
pub struct FitOrdinalRidgeNode {}

impl FitOrdinalRidgeNode {
    pub fn new() -> Self {
        FitOrdinalRidgeNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOrdinalRidgeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_ordinal_ridge",
            "Train Ordinal Model (Ridge)",
            "Fit/Train an ordinal model the cheap way: ridge-regress the level rank on the features, then cut the score at thresholds learned from the training distribution instead of rounding it. Closed-form, so it stays fast exactly where the proportional-odds model gets expensive - many levels, many features, or when you just want a quick ordinal baseline to beat. It also degrades gracefully when the proportional-odds assumption does not hold. Unlike the proportional-odds model it yields no probabilities: you get the predicted level and the latent score behind it, nothing calibrated.",
            "AI/ML/Ordinal",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7) // Closed form, but the solve is O(features^3)
                .set_governance(8) // One coefficient per feature, sign included, is inspectable
                .set_reliability(8) // No iterative solver to stall
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins ordinal ridge training",
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
            "Level labels from LOWEST to HIGHEST, comma separated - e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is already the one you want (`1, 2, 10` sorts as numbers, not as text). Non-numeric labels carry no inferable order, so training fails rather than guessing unless you list them here.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "alpha",
            "Alpha (L2 Penalty)",
            "Strength of the L2 penalty. Must be strictly greater than 0: the penalty is added to the diagonal of the normal equations and is the only thing keeping them positive definite, so the Cholesky solve has a unique answer even with collinear or wide features. Larger values shrink the coefficients harder.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained ordinal ridge model. Predictions come back as the original level labels.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "levels",
            "Levels",
            "The resolved level order the model was trained on, lowest first, plus whether that order came from `Class Order` or from reading the labels as numbers. Check it before trusting the model - a wrong order trains a wrong model without ever failing.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalLevels>();

        node.add_output_pin(
            "coefficients",
            "Coefficients",
            "Fitted coefficients and intercept on the rank scale. The SIGN tells you which way a feature pushes the level: positive moves samples toward the higher levels, negative toward the lower ones. The magnitude is only comparable across features when they share a scale.",
            VariableType::Struct,
        )
        .set_schema::<LinearCoefficients>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let alpha: f64 = context.evaluate_pin("alpha").await?;

        if !alpha.is_finite() || alpha <= 0.0 {
            return Err(anyhow!(
                "`Alpha (L2 Penalty)` must be a finite value strictly greater than 0, got {alpha}. The penalty is added to the diagonal of the normal equations and is what makes them positive definite; at 0 or below the Cholesky solve has no unique solution and the fit is rejected."
            ));
        }

        let explicit_order = parse_class_order(&class_order);

        let t0 = std::time::Instant::now();
        let (train_array, ranks, classes, levels, targets_col) = match source.as_str() {
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
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );
                if records.is_empty() {
                    return Err(anyhow!(
                        "Database returned no rows, there is nothing to train an ordinal model on"
                    ));
                }

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (ranks, classes, levels) = values_to_array1_ordinal(
                    &records,
                    &targets_col,
                    (!explicit_order.is_empty()).then_some(explicit_order.as_slice()),
                )?;
                (train_array, ranks, classes, levels, targets_col)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        // A silently wrong order trains a plausible-looking but meaningless model, so the resolved
        // order is surfaced at Info rather than Debug.
        context.log_message(
            &format!(
                "Ordinal levels ({}), lowest first: {}",
                match levels.ordering {
                    OrdinalOrdering::Explicit => "from `Class Order`",
                    OrdinalOrdering::Numeric => "read as numbers",
                },
                levels.labels.join(" < ")
            ),
            LogLevel::Info,
        );

        let (n_samples, n_features) = train_array.dim();
        if n_features == 0 {
            return Err(anyhow!(
                "Training vectors are empty, the ordinal ridge solve needs at least one feature"
            ));
        }
        if let Some((row, col)) = first_non_finite(&train_array) {
            return Err(anyhow!(
                "Row {row}, feature {col} is not finite (NaN or Inf). The ridge solve requires finite features - a single bad cell makes the whole Gram matrix unusable."
            ));
        }

        let observed: BTreeSet<usize> = ranks.iter().copied().collect();
        if observed.len() < 2 {
            let present = observed
                .iter()
                .map(|rank| match classes.get(rank) {
                    Some(label) => format!("`{label}`"),
                    None => rank.to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow!(
                "Target col `{targets_col}` holds {} distinct level(s): {present}. An ordinal model needs at least 2 levels to have an order to learn.",
                observed.len()
            ));
        }

        if n_features > WIDE_FEATURE_WARNING {
            context.log_message(
                &format!(
                    "Training on {n_features} features: the solve builds a {n_features} x {n_features} Gram matrix (~{:.1} MB of f64) and Cholesky-factorizes it (~{:.1e} operations). Embedding columns are commonly this wide - reduce the width (PCA or feature selection) if the fit is slow or memory bound.",
                    (n_features * n_features * 8) as f64 / (1024.0 * 1024.0),
                    (n_features as f64).powi(3) / 3.0
                ),
                LogLevel::Warn,
            );
        }

        // The rank space is the full level list, not just the levels that happen to appear: an
        // explicit order may name levels the training sample never reached, and the thresholds have
        // to keep a slot for them.
        let n_classes = levels.labels.len();

        let t0 = std::time::Instant::now();
        let dataset = DatasetBase::new(train_array, ranks);
        let fitted = OrdinalRidge::params()
            .alpha(alpha)
            .n_levels(n_classes)
            .fit(&dataset)
            .map_err(|err| anyhow!("Ordinal ridge fit failed: {err}"))?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        let coefficients: Vec<f64> = fitted.coefficients().to_vec();
        let intercept = fitted.intercept();
        context.log_message(
            &format!(
                "Fitted on {n_samples} samples x {n_features} features across {n_classes} levels (alpha {alpha}), intercept {intercept:.6}, cut points {:?}",
                fitted.thresholds().to_vec()
            ),
            LogLevel::Debug,
        );

        let coefficient_result = LinearCoefficients {
            n_features: coefficients.len(),
            coefficients,
            intercept,
        };

        let model = MLModel::OrdinalRidge(ModelWithMeta {
            model: fitted,
            classes: Some(classes),
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("levels", json!(levels)).await?;
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
                    "Column Containing the Ordered Level of Each Row (the ordinal target)",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
