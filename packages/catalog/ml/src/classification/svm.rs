//! Node for Fitting Support Vector Machines (SVM) for Multi-Class Classification
//!
//! This node loads a dataset (currently from a Database), transforms it into a classification dataset,
//! and fits multiple SVM-models using the [`linfa`] crate.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, POLYNOMIAL_KERNEL_CONSTANT,
    validate_polynomial_degree, values_to_array1_target, values_to_array2_f64,
};
#[cfg(feature = "execute")]
use flow_like::flow::{board::Board, execution::LogLevel};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::{prelude::Pr, traits::Fit};
#[cfg(feature = "execute")]
use linfa_svm::{Svm, SvmError, SvmParams};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Constant term of the polynomial kernel `(<x, x'> + c)^degree`. Fixed so a single kernel
/// parameter pin can serve all three kernels.
#[cfg(feature = "execute")]
/// The SMO solver materialises a dense `n x n` kernel matrix per class, so training cost grows
/// quadratically with the number of rows.
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
pub struct FitSVMMultiClassNode {}

impl FitSVMMultiClassNode {
    pub fn new() -> Self {
        FitSVMMultiClassNode {}
    }
}

#[async_trait]
impl NodeLogic for FitSVMMultiClassNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_svm_multi_class",
            "Train Classifier (SVM)",
            "Fit/Train Support Vector Machines (SVM) for Multi-Class Classification ",
            "AI/ML/Classification",
        );
        node.set_flowscript_name("ml", "fitSvmMultiClass");
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins SVM training",
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
                .set_valid_values(vec!["Database".to_string()]) // , "CSV".to_string()
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "kernel",
            "Kernel",
            "Feature-space mapping. Gaussian separates non-linear classes, Linear is the plain SVM, Polynomial adds interaction terms.",
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
            "Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means smoother boundaries. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 1000.0)).build())
        .set_default_value(Some(json!(30.0)));

        node.add_input_pin(
            "c",
            "C",
            "Penalty for misclassified training rows, applied to both the positive and the negative side. Higher values fit the training data harder and risk overfitting.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0001, 100000.0)).build())
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
            "Thread-safe handle to the trained SVM classifier",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let kernel: String = context.evaluate_pin("kernel").await?;
        let kernel_param: f64 = context.evaluate_pin("kernel_param").await?;
        let c: f64 = context.evaluate_pin("c").await?;

        // linfa panics on an invalid parameter combination instead of returning an error, so the
        // hyperparameters are validated before they reach the solver.
        if !is_positive(c) {
            return Err(anyhow!("C must be a finite value > 0, got {c}"));
        }

        // load dataset
        let t0 = std::time::Instant::now();
        let (ds, classes) = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let targets_col: String = context.evaluate_pin("targets").await?;

                // fetch records
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
                }; // drop db
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records, &targets_col)?;
                (
                    DatasetBase::from(train_array).with_targets(target_array),
                    classes,
                )
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let n_samples = ds.records().nrows();
        if n_samples < 2 {
            return Err(anyhow!(
                "SVM classification needs at least 2 training rows, got {n_samples}"
            ));
        }
        let n_classes = ds.targets.iter().copied().collect::<HashSet<usize>>().len();
        if n_classes < 2 {
            return Err(anyhow!(
                "SVM classification needs at least 2 distinct classes in the target col, got {n_classes}"
            ));
        }
        if n_samples > DENSE_KERNEL_WARN_ROWS {
            context.log_message(
                &format!(
                    "Training {n_classes} one-vs-all models on {n_samples} rows: each builds a dense {n_samples}x{n_samples} kernel matrix, expect high memory usage"
                ),
                LogLevel::Warn,
            );
        }

        // train model
        let t0 = std::time::Instant::now();
        let params = apply_kernel(
            Svm::<f64, Pr>::params().pos_neg_weights(c, c),
            &kernel,
            kernel_param,
        )?;
        let mut svm_models: Vec<(usize, Svm<f64, Pr>)> = Vec::with_capacity(n_classes);
        for (class_id, subset) in ds.one_vs_all()? {
            let fitted: std::result::Result<Svm<f64, Pr>, SvmError> = params.fit(&subset);
            let fitted =
                fitted.map_err(|err| anyhow!("SVM fit failed for class {class_id}: {err}"))?;
            let summary = fitted.to_string();
            if summary.starts_with(NON_CONVERGED_PREFIX) {
                context.log_message(
                    &format!("SVM for class {class_id} hit the iteration cap: {summary}"),
                    LogLevel::Warn,
                );
            }
            svm_models.push((class_id, fitted));
        }
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        // set outputs
        let model = MLModel::SVMMultiClass(ModelWithMeta {
            model: svm_models,
            classes,
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
                    "Column Containing the Target Values to Fit the Classifier on",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
            return;
        }
    }
}
