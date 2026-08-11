//! Node for Fitting a **One-Class SVM** for Novelty / Outlier Detection
//!
//! This node loads a dataset (currently from a database source) that is expected to contain only
//! *normal* observations and fits the one-class formulation of the [`linfa_svm`] crate around them.
//! The resulting model answers "does this row look like the training data?" — the Predict node
//! writes `1` for an inlier and `0` for an outlier.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, POLYNOMIAL_KERNEL_CONSTANT,
    validate_polynomial_degree, values_to_array2_f64,
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
use linfa::{prelude::Pr, traits::Fit};
#[cfg(feature = "execute")]
use linfa_svm::{Svm, SvmError, SvmParams};
#[cfg(feature = "execute")]
use ndarray::{Array1, Array2};
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
pub struct FitOneClassSVMNode {}

impl FitOneClassSVMNode {
    pub fn new() -> Self {
        FitOneClassSVMNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOneClassSVMNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_one_class_svm",
            "Fit Novelty Detection (One-Class SVM)",
            "Fit a One-Class SVM on normal observations only. Predictions flag whether a new row is an inlier (1) or an outlier (0).",
            "AI/ML/Classification",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(4) // dense kernel matrix, quadratic in the number of rows
                .set_governance(5)
                .set_reliability(6)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins One-Class SVM training",
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
            "nu",
            "Nu",
            "Upper bound on the fraction of training rows the model is allowed to treat as outliers, in (0, 1]. Raise it when the training set is known to be contaminated.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 1.0)).build())
        .set_default_value(Some(json!(0.1)));

        node.add_input_pin(
            "kernel",
            "Kernel",
            "Feature-space mapping. Gaussian wraps a tight non-linear boundary around the data, Linear yields a half-space, Polynomial adds interaction terms.",
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
            "Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means a looser boundary. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 1000.0)).build())
        .set_default_value(Some(json!(30.0)));

        node.add_input_pin(
            "tolerance",
            "Solver Tolerance",
            "Stopping threshold of the SMO solver. Smaller values train longer for a more precise boundary.",
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
            "Thread-safe handle to the trained One-Class SVM",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "support_vectors",
            "Support Vectors",
            "Number of training rows that define the learned boundary",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let nu: f64 = context.evaluate_pin("nu").await?;
        let kernel: String = context.evaluate_pin("kernel").await?;
        let kernel_param: f64 = context.evaluate_pin("kernel_param").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;

        // The one-class fit multiplies nu by the sample count and unwraps the conversion to usize,
        // so an out-of-range nu would panic inside linfa rather than return an error.
        if !is_positive(nu) || nu > 1.0 {
            return Err(anyhow!("Nu must be in (0, 1], got {nu}"));
        }
        if !is_positive(tolerance) {
            return Err(anyhow!(
                "Solver tolerance must be a finite value > 0, got {tolerance}"
            ));
        }

        let t0 = std::time::Instant::now();
        let train_array: Array2<f64> = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;

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
                    database
                        .filter(
                            "true",
                            Some(vec![records_col.to_string()]),
                            MAX_ML_PREDICTION_RECORDS,
                            0,
                        )
                        .await?
                };
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );

                values_to_array2_f64(&records, &records_col)?
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let n_samples = train_array.nrows();
        if n_samples < 2 {
            return Err(anyhow!(
                "One-Class SVM needs at least 2 training rows, got {n_samples}"
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
        // The unit targets pick the one-class `Fit` impl; a bool/Pr target would train a binary
        // classifier instead.
        let ds: DatasetBase<Array2<f64>, Array1<()>> = DatasetBase::from(train_array);
        let params = apply_kernel(
            Svm::<f64, Pr>::params().nu_weight(nu),
            &kernel,
            kernel_param,
        )?
        .eps(tolerance);

        let fitted: std::result::Result<Svm<f64, bool>, SvmError> = params.fit(&ds);
        let svm_model = fitted.map_err(|err| anyhow!("One-Class SVM fit failed: {err}"))?;
        let elapsed = t0.elapsed();

        let summary = svm_model.to_string();
        if summary.starts_with(NON_CONVERGED_PREFIX) {
            context.log_message(
                &format!(
                    "One-Class SVM hit the iteration cap before reaching the tolerance {tolerance}: {summary}"
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

        // Inlier/outlier is not a learned class mapping, the Predict node names both sides itself.
        let model = MLModel::OneClassSVM(ModelWithMeta {
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
                    "Column containing the feature vectors of the normal observations",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
