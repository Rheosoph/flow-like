//! Node for Fitting a **Support Vector Regressor (SVR)**
//!
//! This node loads a dataset (currently from a database source), transforms it into a regression
//! dataset and fits either epsilon-SVR or nu-SVR using the [`linfa_svm`] crate. Unlike the linear
//! regressors in this catalog, SVR learns non-linear relations through its kernel.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, POLYNOMIAL_KERNEL_CONSTANT,
    validate_polynomial_degree, values_to_array1_f64, values_to_array2_f64,
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
use linfa_svm::{Svm, SvmError, SvmParams};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Constant term of the polynomial kernel `(<x, x'> + c)^degree`. Fixed so a single kernel
/// parameter pin can serve all three kernels.
#[cfg(feature = "execute")]
/// The SMO solver materialises a dense `n x n` kernel matrix, so training cost grows quadratically.
#[cfg(feature = "execute")]
const DENSE_KERNEL_WARN_ROWS: usize = 5000;

/// Upstream prints this prefix when the solver stopped on the iteration cap instead of the
/// tolerance. The exit reason itself is a private field, so the rendered summary is the only signal.
#[cfg(feature = "execute")]
const NON_CONVERGED_PREFIX: &str = "Reached maximal iterations";

#[cfg(feature = "execute")]
fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(feature = "execute")]
fn apply_kernel<T>(
    params: SvmParams<f64, T>,
    kernel: &str,
    kernel_param: f64,
) -> Result<SvmParams<f64, T>> {
    match kernel {
        "Gaussian" => {
            if !is_positive(kernel_param) {
                return Err(anyhow!(
                    "Gaussian kernel parameter must be a finite value > 0, got {kernel_param}"
                ));
            }
            Ok(params.gaussian_kernel(kernel_param))
        }
        "Linear" => Ok(params.linear_kernel()),
        "Polynomial" => {
            validate_polynomial_degree(kernel_param)?;
            Ok(params.polynomial_kernel(POLYNOMIAL_KERNEL_CONSTANT, kernel_param))
        }
        other => Err(anyhow!(
            "Unknown kernel `{other}`. Expected `Gaussian`, `Linear` or `Polynomial`"
        )),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FitSVMRegressionNode {}

impl FitSVMRegressionNode {
    pub fn new() -> Self {
        FitSVMRegressionNode {}
    }
}

#[async_trait]
impl NodeLogic for FitSVMRegressionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_svm_regression",
            "Train Regressor (SVM)",
            "Fit/Train a Support Vector Regressor. Learns non-linear targets through a kernel, with epsilon-SVR or nu-SVR.",
            "AI/ML/Regression",
        );
        node.set_flowscript_name("ml", "fitSvmRegression");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(4) // dense kernel matrix, quadratic in the number of rows
                .set_governance(5)
                .set_reliability(7)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins SVR training",
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
            "mode",
            "Mode",
            "Epsilon-SVR penalises deviations larger than Epsilon. Nu-SVR replaces Epsilon with Nu, the target fraction of support vectors.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Epsilon-SVR".to_string(), "Nu-SVR".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Epsilon-SVR")));

        node.add_input_pin(
            "kernel",
            "Kernel",
            "Feature-space mapping. Gaussian for smooth non-linear targets, Linear for the plain SVR, Polynomial for interaction terms.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Gaussian".to_string(),
                    "Linear".to_string(),
                    "Polynomial".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Gaussian")));

        node.add_input_pin(
            "kernel_param",
            "Kernel Parameter",
            "Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means smoother. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 1000.0)).build())
        .set_default_value(Some(json!(30.0)));

        node.add_input_pin(
            "c",
            "C",
            "Penalty for deviations outside the tolerated margin. Higher values fit the training data harder and risk overfitting. Used by both modes.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 100000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "epsilon",
            "Epsilon",
            "Width of the insensitive tube: errors smaller than this are not penalised. Epsilon-SVR only.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 100.0)).build())
        .set_default_value(Some(json!(0.1)));

        node.add_input_pin(
            "nu",
            "Nu",
            "Upper bound on the fraction of training errors and lower bound on the fraction of support vectors, in (0, 1]. Nu-SVR only.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 1.0)).build())
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "tolerance",
            "Solver Tolerance",
            "Stopping threshold of the SMO solver. Smaller values train longer for a more precise solution.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0000001, 1.0)).build())
        .set_default_value(Some(json!(0.001)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained support vector regressor",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "support_vectors",
            "Support Vectors",
            "Number of training rows that ended up contributing to the regression",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let mode: String = context.evaluate_pin("mode").await?;
        let kernel: String = context.evaluate_pin("kernel").await?;
        let kernel_param: f64 = context.evaluate_pin("kernel_param").await?;
        let c: f64 = context.evaluate_pin("c").await?;
        let epsilon: f64 = context.evaluate_pin("epsilon").await?;
        let nu: f64 = context.evaluate_pin("nu").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;

        // linfa panics on an unset/invalid parameter combination instead of returning an error,
        // so every hyperparameter is validated before it reaches the solver.
        if !is_positive(c) {
            return Err(anyhow!("C must be a finite value > 0, got {c}"));
        }
        if !is_positive(tolerance) {
            return Err(anyhow!(
                "Solver tolerance must be a finite value > 0, got {tolerance}"
            ));
        }

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
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let n_samples = ds.records().nrows();
        if n_samples < 2 {
            return Err(anyhow!(
                "SVM regression needs at least 2 training rows, got {n_samples}"
            ));
        }
        if n_samples > DENSE_KERNEL_WARN_ROWS {
            context.log_message(
                &format!(
                    "Training on {n_samples} rows: the solver builds a dense {n_samples}x{n_samples} kernel matrix, expect high memory usage"
                ),
                LogLevel::Warn,
            );
        }

        let t0 = std::time::Instant::now();
        let params = Svm::<f64, f64>::params();
        let params = match mode.as_str() {
            "Epsilon-SVR" => {
                if !is_positive(epsilon) {
                    return Err(anyhow!("Epsilon must be a finite value > 0, got {epsilon}"));
                }
                params.c_svr(c, Some(epsilon))
            }
            "Nu-SVR" => {
                if !is_positive(nu) || nu > 1.0 {
                    return Err(anyhow!("Nu must be in (0, 1], got {nu}"));
                }
                params.nu_svr(nu, Some(c))
            }
            other => {
                return Err(anyhow!(
                    "Unknown mode `{other}`. Expected `Epsilon-SVR` or `Nu-SVR`"
                ));
            }
        };
        let params = apply_kernel(params, &kernel, kernel_param)?.eps(tolerance);

        let fitted: std::result::Result<Svm<f64, f64>, SvmError> = params.fit(&ds);
        let svm_model = fitted.map_err(|err| anyhow!("SVM regression fit failed: {err}"))?;
        let elapsed = t0.elapsed();

        let summary = svm_model.to_string();
        if summary.starts_with(NON_CONVERGED_PREFIX) {
            context.log_message(
                &format!(
                    "SVM regression hit the iteration cap before reaching the tolerance {tolerance}: {summary}"
                ),
                LogLevel::Warn,
            );
        }
        context.log_message(
            &format!("Fit model: {elapsed:?} ({summary})"),
            LogLevel::Debug,
        );

        context
            .set_pin_value("support_vectors", json!(svm_model.nsupport() as i64))
            .await?;

        let model = MLModel::SVMRegression(ModelWithMeta {
            model: svm_model,
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
                    "Column Containing the Values to Train on",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("targets").is_none() {
                node.add_input_pin(
                    "targets",
                    "Target Col",
                    "Column Containing the Continuous Target Values to Fit the Regressor on",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
