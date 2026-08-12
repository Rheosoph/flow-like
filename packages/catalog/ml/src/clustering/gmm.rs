//! Node for Fitting a **Gaussian Mixture Model (GMM)**
//!
//! This node loads a dataset (currently from a database source), transforms it into a
//! clustering dataset, and fits a Gaussian Mixture Model using the [`linfa_clustering`] crate.
//! Unlike KMeans, a GMM is a *soft* clustering: every component carries a full covariance
//! matrix and a mixture weight, so points get probabilistic memberships instead of a hard
//! nearest-centroid assignment.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array2_f64};
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
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
#[cfg(feature = "execute")]
use flow_like_types::rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_clustering::{GaussianMixtureModel, GmmCovarType, GmmError, GmmInitMethod};
#[cfg(feature = "execute")]
use ndarray::{Array2, Axis};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Upper bound on `k * d^2` covariance cells accepted for a single fit.
///
/// A full-covariance GMM keeps `covariances`, `precisions` and `precisions_chol` (3 arrays of
/// `k * d^2` f64) plus a full clone of the best model, and allocates one more array while
/// inverting the Cholesky factor - roughly 7 arrays, i.e. ~56 bytes per cell. 4M cells is
/// therefore ~225 MB of peak matrix memory, which is the most a single node run may spend.
/// Embedding columns (384-1536 dims) blow past this at any usable cluster count, which is the
/// point: they must be reduced with the PCA node before a GMM is meaningful anyway.
#[cfg(feature = "execute")]
const MAX_GMM_COVARIANCE_CELLS: usize = 4_000_000;

/// linfa 0.8 pins the GMM RNG to `Xoshiro256Plus::seed_from_u64(42)` in `params()`.
#[cfg(feature = "execute")]
const LINFA_GMM_SEED: i64 = 42;

#[crate::register_node]
#[derive(Default)]
pub struct FitGaussianMixtureNode {}

impl FitGaussianMixtureNode {
    pub fn new() -> Self {
        FitGaussianMixtureNode {}
    }
}

#[async_trait]
impl NodeLogic for FitGaussianMixtureNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_gaussian_mixture",
            "Fit Clustering (Gaussian Mixture)",
            "Fit/Train a Gaussian Mixture Model. Soft clustering with per-component covariances and mixture weights, fitted by Expectation-Maximization.",
            "AI/ML/Clustering",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(4)
                .set_governance(6)
                .set_reliability(5)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins Gaussian Mixture training",
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
            "n_clusters",
            "Components",
            "Number of Gaussian components (k) in the mixture. Each component costs a full d x d covariance matrix.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 100.)).build())
        .set_default_value(Some(json!(3)));

        node.add_input_pin(
            "covariance_type",
            "Covariance Type",
            "Shape of each component's covariance. linfa 0.8 implements full covariances only - scikit-learn's diag, tied and spherical variants do not exist here, so every component always costs d x d parameters.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Full".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Full")));

        node.add_input_pin(
            "init_method",
            "Init Method",
            "How initial responsibilities are built: KMeans runs a KMeans pass first (usually the better optimum), Random draws them uniformly.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["KMeans".to_string(), "Random".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("KMeans")));

        node.add_input_pin(
            "n_runs",
            "Runs",
            "Number of EM passes. Note: linfa 0.8 continues each pass from the previous parameters instead of re-initializing, so this multiplies the iteration budget (Runs x Max Iterations) rather than performing independent restarts. Vary the Seed for a genuinely different start.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 50.)).build())
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "EM stops once the average log-likelihood gain per iteration falls below this value",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.000_000_1, 1.0)).build())
        .set_default_value(Some(json!(0.001)));

        node.add_input_pin(
            "reg_covariance",
            "Reg Covariance",
            "Non-negative value added to each covariance diagonal to keep it positive definite. Raise it when the fit reports a singular covariance; 0 makes duplicate or constant rows fail outright.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(0.000_001)));

        node.add_input_pin(
            "max_n_iterations",
            "Max Iterations",
            "Maximum number of EM iterations per run",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 10000.)).build())
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "seed",
            "Seed",
            "Seed for the training row order. linfa 0.8 hard-codes its internal RNG (seed 42) and exposes no seeding hook on this entry point, so changing the seed re-orders the rows, which is what changes the initial responsibilities. Keep 42 to reproduce linfa's stock ordering.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(42)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained Gaussian Mixture model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "weights",
            "Mixture Weights",
            "Fitted mixture proportions, one per component, summing to 1. A tiny weight means that component captured almost no data.",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let n_clusters: i64 = context.evaluate_pin("n_clusters").await?;
        let covariance_type: String = context.evaluate_pin("covariance_type").await?;
        let init_method: String = context.evaluate_pin("init_method").await?;
        let n_runs: i64 = context.evaluate_pin("n_runs").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;
        let reg_covariance: f64 = context.evaluate_pin("reg_covariance").await?;
        let max_n_iterations: i64 = context.evaluate_pin("max_n_iterations").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;

        if n_clusters < 1 {
            return Err(anyhow!(
                "Components must be at least 1, got {n_clusters}. Every component is one Gaussian in the mixture."
            ));
        }
        if n_runs < 1 {
            return Err(anyhow!("Runs must be at least 1, got {n_runs}"));
        }
        if max_n_iterations < 1 {
            return Err(anyhow!(
                "Max Iterations must be at least 1, got {max_n_iterations}"
            ));
        }
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(anyhow!(
                "Tolerance must be a finite value greater than 0, got {tolerance}"
            ));
        }
        if !reg_covariance.is_finite() || reg_covariance < 0.0 {
            return Err(anyhow!(
                "Reg Covariance must be a finite, non-negative value, got {reg_covariance}"
            ));
        }

        let covar_type = match covariance_type.as_str() {
            "Full" => GmmCovarType::Full,
            other => {
                return Err(anyhow!(
                    "Unknown covariance type `{other}`. linfa 0.8 only implements `Full`."
                ));
            }
        };
        let init = match init_method.as_str() {
            "KMeans" => GmmInitMethod::KMeans,
            "Random" => GmmInitMethod::Random,
            other => {
                return Err(anyhow!(
                    "Unknown init method `{other}`. Expected `KMeans` or `Random`."
                ));
            }
        };

        let n_clusters = n_clusters as usize;

        let t0 = std::time::Instant::now();
        let (array, records_col) = match source.as_str() {
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
                    &format!("Loaded {} records from database", records.len()),
                    LogLevel::Debug,
                );

                (values_to_array2_f64(&records, &records_col)?, records_col)
            }
            _ => return Err(anyhow!("Datasource Not Implemented")),
        };
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        let (n_samples, n_features) = array.dim();
        if n_samples == 0 {
            return Err(anyhow!(
                "Column `{records_col}` yielded no rows to cluster on"
            ));
        }
        if n_samples < n_clusters {
            return Err(anyhow!(
                "Cannot fit {n_clusters} components on {n_samples} samples. Lower the component count to at most {n_samples}."
            ));
        }

        let covariance_cells = n_clusters
            .saturating_mul(n_features)
            .saturating_mul(n_features);
        if covariance_cells > MAX_GMM_COVARIANCE_CELLS {
            let estimated_mb = covariance_cells.saturating_mul(56) / (1024 * 1024);
            let max_dims = ((MAX_GMM_COVARIANCE_CELLS / n_clusters) as f64).sqrt() as usize;
            return Err(anyhow!(
                "Gaussian Mixture would need {n_clusters} x {n_features}^2 = {covariance_cells} covariance cells (~{estimated_mb} MB of matrices), above the {MAX_GMM_COVARIANCE_CELLS} cell budget. Column `{records_col}` is too wide for a full-covariance GMM - reduce it with the PCA Reduction node first (at most {max_dims} dimensions for {n_clusters} components) or lower the component count."
            ));
        }

        let samples_per_cluster = n_samples / n_clusters;
        if samples_per_cluster <= n_features {
            context.log_message(
                &format!(
                    "Gaussian Mixture is under-determined: ~{samples_per_cluster} samples per component for {n_features} dimensions. A full covariance needs more than {n_features} points per component to be non-singular, so the fit leans entirely on Reg Covariance and may fail. Reduce dimensionality (PCA node) or lower the component count."
                ),
                LogLevel::Warn,
            );
        }
        if reg_covariance == 0.0 {
            context.log_message(
                "Reg Covariance is 0: the Cholesky decomposition will fail on duplicate or constant rows.",
                LogLevel::Warn,
            );
        }
        if n_runs > 1 {
            context.log_message(
                &format!(
                    "Runs={n_runs} extends the EM budget to {} iterations; linfa resumes from the previous parameters instead of restarting.",
                    n_runs.saturating_mul(max_n_iterations)
                ),
                LogLevel::Debug,
            );
        }

        // linfa fixes the GMM RNG internally, so the row order is the only initialization knob we
        // can turn; leave it untouched for the stock seed to keep the default path allocation-free.
        let array = if seed == LINFA_GMM_SEED {
            array
        } else {
            permute_rows(array, seed as u64)
        };

        let t0 = std::time::Instant::now();
        let dataset = DatasetBase::from(array);
        let params = GaussianMixtureModel::<f64>::params(n_clusters)
            .covariance_type(covar_type)
            .init_method(init)
            .n_runs(n_runs as u64)
            .tolerance(tolerance)
            .reg_covariance(reg_covariance)
            .max_n_iterations(max_n_iterations as u64);
        let fitted = match params.fit(&dataset) {
            Ok(fitted) => fitted,
            Err(err) => {
                return Err(describe_fit_error(
                    err,
                    n_clusters,
                    n_features,
                    samples_per_cluster,
                ));
            }
        };
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        let weights: Vec<f64> = fitted.weights().to_vec();
        context.log_message(&format!("Mixture weights: {weights:?}"), LogLevel::Debug);
        let min_weight = weights.iter().copied().fold(f64::INFINITY, f64::min);
        if min_weight * n_samples as f64 <= n_features as f64 {
            context.log_message(
                &format!(
                    "Weakest component holds only ~{:.0} of {n_samples} samples for {n_features} dimensions; its covariance is effectively singular and its predictions are unreliable.",
                    min_weight * n_samples as f64
                ),
                LogLevel::Warn,
            );
        }

        let model = MLModel::GaussianMixture(ModelWithMeta {
            model: fitted,
            classes: None,
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("weights", json!(weights)).await?;
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
                    "Column containing the feature vectors to cluster. Full covariances scale with dimensions squared, so point this at a reduced column (PCA output), not a raw embedding.",
                    VariableType::String,
                )
                .set_default_value(Some(json!("pca_vector")));
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}

#[cfg(feature = "execute")]
fn permute_rows(array: Array2<f64>, seed: u64) -> Array2<f64> {
    let mut order: Vec<usize> = (0..array.nrows()).collect();
    let mut rng = StdRng::seed_from_u64(seed);
    order.shuffle(&mut rng);
    array.select(Axis(0), &order)
}

#[cfg(feature = "execute")]
fn describe_fit_error(
    err: GmmError,
    n_clusters: usize,
    n_features: usize,
    samples_per_cluster: usize,
) -> flow_like_types::Error {
    let hint = match &err {
        GmmError::EmptyCluster(_) => format!(
            "A component lost all of its points. Lower the component count below {n_clusters} or switch Init Method to KMeans."
        ),
        GmmError::LinalgError(_) => format!(
            "A component's covariance is singular. A full covariance needs more than {n_features} points per component (~{samples_per_cluster} here): reduce dimensionality with the PCA Reduction node, lower the component count, or raise Reg Covariance."
        ),
        GmmError::NotConverged(_) => {
            "EM never reached the tolerance. Raise Max Iterations, raise Tolerance, or lower the component count.".to_string()
        }
        GmmError::KMeansError(_) => {
            "The KMeans initialization failed. Switch Init Method to Random or lower the component count.".to_string()
        }
        _ => "Check the component count, the tolerance and the training column.".to_string(),
    };
    anyhow!("Gaussian Mixture fit failed: {err} | {hint}")
}
