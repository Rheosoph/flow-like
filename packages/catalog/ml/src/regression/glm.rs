//! Node for Fitting a **Generalized Linear Model** (Tweedie family).
//!
//! The Tweedie power selects the underlying target distribution (Normal, Poisson, Gamma, Inverse
//! Gaussian, …) and therefore the loss that [`linfa_linear`] optimizes with L-BFGS.

use crate::ml::NodeMLModel;
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
use linfa_linear::TweedieRegressor;
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct FitGlmNode {}

impl FitGlmNode {
    pub fn new() -> Self {
        FitGlmNode {}
    }
}

#[async_trait]
impl NodeLogic for FitGlmNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_glm",
            "Train Regressor (GLM / Tweedie)",
            "Fit/Train a Generalized Linear Model. Pick the distribution that matches the target: Normal for unbounded values, Poisson for counts, Gamma for positive skewed amounts, Inverse Gaussian for heavy tails.",
            "AI/ML/Regression",
        );
        node.set_flowscript_name("ml", "fitGlm");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(6)
                .set_governance(8) // Linear coefficients stay interpretable
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins GLM training",
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
            "distribution",
            "Distribution",
            "Target distribution: Normal (power 0, any value), Poisson (power 1, counts >= 0), Gamma (power 2, values > 0), Inverse Gaussian (power 3, values > 0), or Custom to set the Tweedie power directly",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Normal".to_string(),
                    "Poisson".to_string(),
                    "Gamma".to_string(),
                    "Inverse Gaussian".to_string(),
                    "Custom".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Normal")));

        node.add_input_pin(
            "power",
            "Tweedie Power",
            "Free Tweedie power, only used when Distribution is Custom. Values in (0, 1) do not describe any distribution and are rejected; (1, 2) is compound Poisson-Gamma.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((-5.0, 5.0)).build())
        .set_default_value(Some(json!(0.0)));

        node.add_input_pin(
            "alpha",
            "Alpha (L2 Penalty)",
            "Strength of the L2 penalty on the coefficients. 0 fits an unpenalized GLM.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "fit_intercept",
            "Fit Intercept",
            "Fit a bias term. Disable only when the data is already centered.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "max_iter",
            "Max Iterations",
            "Iteration cap for the L-BFGS solver. Defaults to 1000 instead of the library default of 100, which is too low to converge on unscaled real-world features.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 1_000_000.0)).build())
        .set_default_value(Some(json!(1000)));

        node.add_input_pin(
            "tol",
            "Tolerance",
            "Gradient tolerance that stops the L-BFGS solver. Smaller values fit tighter but need more iterations.",
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
            "Thread-safe handle to the trained generalized linear model",
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
        let distribution: String = context.evaluate_pin("distribution").await?;
        let custom_power: f64 = context.evaluate_pin("power").await?;
        let alpha: f64 = context.evaluate_pin("alpha").await?;
        let fit_intercept: bool = context.evaluate_pin("fit_intercept").await?;
        let max_iter: i64 = context.evaluate_pin("max_iter").await?;
        let tol: f64 = context.evaluate_pin("tol").await?;

        let power = match distribution.as_str() {
            "Normal" => 0.0,
            "Poisson" => 1.0,
            "Gamma" => 2.0,
            "Inverse Gaussian" => 3.0,
            "Custom" => custom_power,
            other => {
                return Err(anyhow!(
                    "Unknown distribution `{other}`, expected `Normal`, `Poisson`, `Gamma`, `Inverse Gaussian` or `Custom`"
                ));
            }
        };

        if !power.is_finite() {
            return Err(anyhow!("`Tweedie Power` must be finite, got {power}"));
        }
        if power > 0.0 && power < 1.0 {
            return Err(anyhow!(
                "No Tweedie distribution exists for power {power}: the interval (0, 1) is undefined. Use 0 (Normal), 1 (Poisson), 2 (Gamma) or 3 (Inverse Gaussian)."
            ));
        }
        if !alpha.is_finite() || alpha < 0.0 {
            return Err(anyhow!(
                "`Alpha (L2 Penalty)` must be a finite value >= 0, got {alpha}"
            ));
        }
        if !tol.is_finite() || tol <= 0.0 {
            return Err(anyhow!("`Tolerance` must be a finite value > 0, got {tol}"));
        }
        if !(1..=u32::MAX as i64).contains(&max_iter) {
            return Err(anyhow!(
                "`Max Iterations` must be between 1 and {}, got {max_iter}",
                u32::MAX
            ));
        }
        let max_iter = max_iter as usize;

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

        // Mirrors `TweedieDistribution::in_range`: powers in [1, 2) accept zero, powers >= 2 do not,
        // and powers <= 0 accept the whole real line. Upstream only reports "some value(s) of y are
        // out of the valid range", so the offending row is resolved here instead.
        if power >= 1.0 {
            let zero_allowed = power < 2.0;
            let offender = ds
                .targets()
                .iter()
                .enumerate()
                .find(|(_, y)| if zero_allowed { **y < 0.0 } else { **y <= 0.0 });
            if let Some((row, value)) = offender {
                let requirement = if zero_allowed { ">= 0" } else { "> 0" };
                return Err(anyhow!(
                    "`{distribution}` (Tweedie power {power}) is out of domain for the training targets: every target must be {requirement}, but row {row} is {value}. Pick a distribution that matches the data or clean the target column."
                ));
            }
        }

        // With an intercept and a positive power, upstream seeds the solver with `ln(mean(y))` of the
        // log link. An all-zero target column would seed L-BFGS with -inf and yield NaN coefficients
        // instead of an error.
        let target_mean = ds
            .targets()
            .mean()
            .ok_or_else(|| anyhow!("Could not compute the mean of the target column"))?;
        if fit_intercept && power > 0.0 && target_mean <= 0.0 {
            return Err(anyhow!(
                "`{distribution}` (Tweedie power {power}) uses a log link, whose intercept is seeded with ln(mean(y)), but the mean of the target column is {target_mean}. Disable `Fit Intercept` or use targets that are not all zero."
            ));
        }

        let t0 = std::time::Instant::now();
        let params = TweedieRegressor::<f64>::params()
            .power(power)
            .alpha(alpha)
            .fit_intercept(fit_intercept)
            .max_iter(max_iter)
            .tol(tol);
        let fitted: TweedieRegressor<f64> = params.fit(&ds)?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        // L-BFGS stops silently at the iteration cap and upstream drops the solver state, so a
        // diverged run can only be detected from the coefficients it left behind.
        if !fitted.intercept.is_finite() || fitted.coef.iter().any(|c| !c.is_finite()) {
            return Err(anyhow!(
                "GLM fit diverged: the solver produced non-finite coefficients for `{distribution}` (Tweedie power {power}). Scale the features, raise `Alpha (L2 Penalty)` or pick a distribution that matches the target."
            ));
        }

        context.log_message(
            &format!(
                "{distribution} GLM (power {power}, alpha {alpha}) fit on {n_samples} samples x {n_features} features, intercept {:.6}",
                fitted.intercept
            ),
            LogLevel::Debug,
        );

        let model = MLModel::TweedieRegressor(ModelWithMeta {
            model: fitted,
            classes: None,
        });
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
                    "Column Containing the Feature Vectors to Train on",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("targets").is_none() {
                node.add_input_pin(
                    "targets",
                    "Target Col",
                    "Column Containing the Numeric Target Values to Fit the GLM on",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
