//! Node for Fitting **Penalized Linear Regression** (Ridge / Lasso / Elastic Net).
//!
//! All three members of the family share one solver in [`linfa_elasticnet`]; the penalty type only
//! decides how the overall penalty budget is split between the L1 and L2 terms.

use crate::ml::{LinearCoefficients, NodeMLModel};
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array1_f64, values_to_array2_f64,
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
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_elasticnet::ElasticNet;
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct FitElasticNetNode {}

impl FitElasticNetNode {
    pub fn new() -> Self {
        FitElasticNetNode {}
    }
}

#[async_trait]
impl NodeLogic for FitElasticNetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_elastic_net",
            "Train Regressor (Ridge/Lasso/ElasticNet)",
            "Fit/Train a penalized linear regression model. Ridge shrinks all coefficients, Lasso drives irrelevant ones to exactly zero (feature selection), Elastic Net mixes both.",
            "AI/ML/Regression",
        );
        node.set_flowscript_name("ml", "fitElasticNet");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(8) // Coefficients are directly inspectable
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins regression training",
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
            "penalty_type",
            "Penalty Type",
            "Ridge = pure L2 (keeps all features, handles correlated ones well), Lasso = pure L1 (zeroes out weak features), ElasticNet = a blend controlled by L1 Ratio",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "ElasticNet".to_string(),
                    "Ridge".to_string(),
                    "Lasso".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("ElasticNet")));

        node.add_input_pin(
            "penalty",
            "Penalty (Alpha)",
            "Overall regularization strength. 0 means ordinary least squares, larger values shrink the coefficients harder.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "l1_ratio",
            "L1 Ratio",
            "Share of the penalty spent on L1 vs L2. Only used when Penalty Type is ElasticNet; Ridge forces 0.0 and Lasso forces 1.0.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "with_intercept",
            "Fit Intercept",
            "Fit a bias term. Disable only when the data is already centered.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "max_iterations",
            "Max Iterations",
            "Upper bound on coordinate descent passes. The solver stops silently at this cap, so a convergence warning is logged when it is hit.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 1_000_000.0)).build())
        .set_default_value(Some(json!(1000)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Convergence tolerance for coordinate descent. Smaller values give a tighter fit at the cost of more iterations.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((1e-12, 1.0)).build())
        .set_default_value(Some(json!(1e-4)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained penalized regression model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "coefficients",
            "Coefficients",
            "Fitted coefficients and intercept. With Lasso, coefficients that are exactly zero mark features the model discarded.",
            VariableType::Struct,
        )
        .set_schema::<LinearCoefficients>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let penalty_type: String = context.evaluate_pin("penalty_type").await?;
        let penalty: f64 = context.evaluate_pin("penalty").await?;
        let l1_ratio: f64 = context.evaluate_pin("l1_ratio").await?;
        let with_intercept: bool = context.evaluate_pin("with_intercept").await?;
        let max_iterations: i64 = context.evaluate_pin("max_iterations").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;

        if !penalty.is_finite() || penalty < 0.0 {
            return Err(anyhow!(
                "`Penalty (Alpha)` must be a finite value >= 0, got {penalty}"
            ));
        }
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(anyhow!(
                "`Tolerance` must be a finite value > 0, got {tolerance}"
            ));
        }
        if !(1..=u32::MAX as i64).contains(&max_iterations) {
            return Err(anyhow!(
                "`Max Iterations` must be between 1 and {}, got {max_iterations}",
                u32::MAX
            ));
        }
        let max_iterations = max_iterations as u32;

        let effective_l1_ratio = match penalty_type.as_str() {
            "Ridge" => 0.0,
            "Lasso" => 1.0,
            "ElasticNet" => {
                if !l1_ratio.is_finite() || !(0.0..=1.0).contains(&l1_ratio) {
                    return Err(anyhow!(
                        "`L1 Ratio` must be between 0.0 and 1.0, got {l1_ratio}"
                    ));
                }
                l1_ratio
            }
            other => {
                return Err(anyhow!(
                    "Unknown penalty type `{other}`, expected `ElasticNet`, `Ridge` or `Lasso`"
                ));
            }
        };

        let t0 = std::time::Instant::now();
        let ds = match source.as_str() {
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

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let target_array = values_to_array1_f64(&records, &targets_col)?;
                DatasetBase::from(train_array).with_targets(target_array)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        // Upstream centers the targets with `mean_axis(...).expect(...)`, which panics on an empty
        // sample axis, and a zero-width record matrix would fit a model with no coefficients.
        let n_samples = ds.records().nrows();
        let n_features = ds.records().ncols();
        if n_samples == 0 {
            return Err(anyhow!("No training records loaded, cannot fit a model"));
        }
        if n_features == 0 {
            return Err(anyhow!(
                "Training records have 0 features, expected at least one value per row"
            ));
        }
        if ds.targets().len() != n_samples {
            return Err(anyhow!(
                "Record/target length mismatch: {n_samples} records but {} targets",
                ds.targets().len()
            ));
        }

        let t0 = std::time::Instant::now();
        let params = match penalty_type.as_str() {
            "Ridge" => ElasticNet::<f64>::ridge(),
            "Lasso" => ElasticNet::<f64>::lasso(),
            _ => ElasticNet::<f64>::params().l1_ratio(effective_l1_ratio),
        }
        .penalty(penalty)
        .with_intercept(with_intercept)
        .max_iterations(max_iterations)
        .tolerance(tolerance);
        let fitted: ElasticNet<f64> = params.fit(&ds)?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        // Coordinate descent stops silently at the iteration cap. Upstream compares the duality gap
        // against `tolerance * y·y` rather than the raw tolerance, and it centers the targets first
        // when an intercept is fitted, so both steps are reproduced here to judge convergence.
        let n_steps = fitted.n_steps();
        let duality_gap = fitted.duality_gap();
        let y_offset = if with_intercept {
            ds.targets().mean().unwrap_or(0.0)
        } else {
            0.0
        };
        let y_sq_norm: f64 = ds
            .targets()
            .iter()
            .map(|y| (y - y_offset) * (y - y_offset))
            .sum();
        let gap_threshold = tolerance * y_sq_norm;
        let converged = duality_gap < gap_threshold;
        if !converged {
            context.log_message(
                &format!(
                    "Coordinate descent did not converge: duality gap {duality_gap:e} exceeds the stopping threshold {gap_threshold:e} (= tolerance {tolerance:e} x ||y||^2 {y_sq_norm:e}) after {n_steps} of {max_iterations} iterations. Raise `Max Iterations`, loosen `Tolerance` or scale the features."
                ),
                LogLevel::Warn,
            );
        }

        let coefficients: Vec<f64> = fitted.hyperplane().to_vec();
        let intercept = fitted.intercept();
        let n_selected = coefficients.iter().filter(|c| **c != 0.0).count();
        context.log_message(
            &format!(
                "{penalty_type} fit on {n_samples} samples x {n_features} features (l1_ratio {effective_l1_ratio}): {n_selected} of {} coefficients are non-zero, intercept {intercept:.6}",
                coefficients.len()
            ),
            LogLevel::Debug,
        );

        let coefficient_result = LinearCoefficients {
            n_features: coefficients.len(),
            coefficients,
            intercept,
        };

        let model = MLModel::ElasticNet(ModelWithMeta {
            model: fitted,
            classes: None,
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("model", json!(node_model)).await?;
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
                    "Column Containing the Numeric Target Values to Fit the Regression Model on",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
