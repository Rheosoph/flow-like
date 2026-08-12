//! Grid Search Hyperparameter Tuning
//!
//! Exhaustive search over parameter grid with cross-validation.

use crate::ml::{GridSearchEntry, GridSearchResult, NodeMLModel, ParameterSpec};
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, PersistedEnsemble, values_to_array1_target,
    values_to_array2_f64,
};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::rand::{self, seq::SliceRandom};
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::composing::MultiClassModel;
#[cfg(feature = "execute")]
use linfa::dataset::Records;
#[cfg(feature = "execute")]
use linfa::prelude::Pr;
#[cfg(feature = "execute")]
use linfa::traits::{Fit, Predict};
#[cfg(feature = "execute")]
use linfa_bayes::GaussianNb;
#[cfg(feature = "execute")]
use linfa_ensemble::RandomForestParams;
#[cfg(feature = "execute")]
use linfa_logistic::{LogisticRegression, MultiLogisticRegression};
#[cfg(feature = "execute")]
use linfa_svm::Svm;
#[cfg(feature = "execute")]
use linfa_trees::DecisionTree as LinfaDecisionTree;
// linfa 0.8 is built against rand 0.8, so the RNG handed to it cannot come from
// `flow_like_types::rand` (0.9) — the `Rng` traits are different types.
#[cfg(feature = "execute")]
use rand_xoshiro::Xoshiro256Plus;
#[cfg(feature = "execute")]
use rand_xoshiro::rand_core::SeedableRng;
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

/// Kernel width shared with the standalone SVM node so a tuned SVM matches what that node trains.
#[cfg(feature = "execute")]
const GAUSSIAN_KERNEL_EPS: f64 = 30.0;

/// Model families this node can tune.
///
/// These are [`MLModel::kind`] strings, which is what makes the Auto Classifier's `best_model_type`
/// output directly feedable into this node's `model_type` input.
#[cfg(feature = "execute")]
const TUNABLE_MODELS: [&str; 5] = [
    "DecisionTree",
    "GaussianNaiveBayes",
    "LogisticRegression",
    "RandomForest",
    "SVMMultiClass",
];

/// Parameter names a model family actually consumes.
///
/// The Parameter Grid pin is seeded once from whichever model type was selected when the node was
/// created and is deliberately never rewritten (that would clobber hand-edited grids). Without
/// this check, switching Model Type afterwards would leave the previous model's parameters in
/// place, and the search would silently score N identical configurations.
#[cfg(feature = "execute")]
fn known_params(model_type: &str) -> &'static [&'static str] {
    match model_type {
        "DecisionTree" => &["max_depth", "min_weight_split"],
        "LogisticRegression" => &["alpha"],
        "RandomForest" => &[
            "ensemble_size",
            "max_depth",
            "min_weight_split",
            "bootstrap_proportion",
            "feature_proportion",
        ],
        _ => &[],
    }
}

/// Default parameter grid for a model family, used to seed the Parameter Grid pin.
fn default_param_grid(model_type: &str) -> Value {
    match model_type {
        "DecisionTree" => json!([
            {"name": "max_depth", "values": [5, 10, 15, 20]},
            {"name": "min_weight_split", "values": [1.0, 2.0, 5.0]}
        ]),
        "LogisticRegression" => json!([
            {"name": "alpha", "values": [0.0, 0.1, 1.0, 10.0]}
        ]),
        "RandomForest" => json!([
            {"name": "ensemble_size", "values": [50, 100, 200]},
            {"name": "max_depth", "values": [5, 10, 15]}
        ]),
        // Neither Gaussian Naive Bayes nor the OvA SVM wrapper exposes a parameter worth sweeping
        // through this node, so they are evaluated as a single configuration.
        _ => json!([]),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GridSearchNode {}

impl GridSearchNode {
    pub fn new() -> Self {
        GridSearchNode {}
    }
}

#[async_trait]
impl NodeLogic for GridSearchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_tuning_grid_search",
            "Grid Search",
            "Exhaustive search over parameter combinations with cross-validation. Returns the best parameters found. Model Type accepts the same names the Auto Classifier reports as its best model, so the two nodes chain directly.",
            "AI/ML/Tuning",
        );
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_version(2);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(4) // Can be slow with many combinations
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5) // Expensive - trains many models
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model_type",
            "Model Type",
            "Type of model to tune",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    [
                        "DecisionTree",
                        "GaussianNaiveBayes",
                        "LogisticRegression",
                        "RandomForest",
                        "SVMMultiClass",
                    ]
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
                )
                .build(),
        )
        .set_default_value(Some(json!("DecisionTree")));

        node.add_input_pin(
            "cv_folds",
            "CV Folds",
            "Number of cross-validation folds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "source",
            "Data Source",
            "Database containing the training data",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated when grid search completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Complete grid search results with all combinations tried",
            VariableType::Struct,
        )
        .set_schema::<GridSearchResult>();

        node.add_output_pin(
            "best_model",
            "Best Model",
            "The model trained with the best parameters on full training data",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use std::time::Instant;

        context.deactivate_exec_pin("exec_out").await?;

        let model_type: String = context.evaluate_pin("model_type").await?;
        let cv_folds: i64 = context.evaluate_pin("cv_folds").await?;
        let source: String = context.evaluate_pin("source").await?;
        let param_grid: Vec<ParameterSpec> = context.evaluate_pin("param_grid").await?;

        // Checked before the cast: a negative value would wrap to usize::MAX and sail past the
        // guard, then blow up in Vec::with_capacity.
        if cv_folds < 2 {
            return Err(flow_like_types::anyhow!(
                "CV folds must be at least 2, got {cv_folds}"
            ));
        }
        let cv_folds = cv_folds as usize;
        if !TUNABLE_MODELS.contains(&model_type.as_str()) {
            return Err(flow_like_types::anyhow!(
                "Unknown model type: `{model_type}`. Supported: {}",
                TUNABLE_MODELS.join(", ")
            ));
        }

        // An empty grid means "use the defaults for this model type", which keeps the node correct
        // when Model Type is switched after the pin was first seeded.
        let param_grid: Vec<ParameterSpec> = if param_grid.is_empty() {
            let defaults: Vec<ParameterSpec> =
                flow_like_types::json::from_value(default_param_grid(&model_type))
                    .unwrap_or_default();
            if !defaults.is_empty() {
                context.log_message(
                    &format!("Parameter Grid is empty, using the default grid for {model_type}"),
                    LogLevel::Info,
                );
            }
            defaults
        } else {
            param_grid
        };

        let start_time = Instant::now();

        // Load data
        let (records, classes, _records_col, _targets_col) = match source.as_str() {
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
                        return Err(flow_like_types::anyhow!(
                            "Database doesn't contain train col `{}`!",
                            records_col
                        ));
                    }
                    if !existing_cols.contains(&targets_col) {
                        return Err(flow_like_types::anyhow!(
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

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records, &targets_col)?;
                (
                    DatasetBase::from(train_array).with_targets(target_array),
                    classes,
                    records_col,
                    targets_col,
                )
            }
            _ => return Err(flow_like_types::anyhow!("Datasource Not Implemented!")),
        };

        context.log_message(
            &format!(
                "Grid Search: {} samples, {} folds",
                records.nsamples(),
                cv_folds
            ),
            LogLevel::Info,
        );

        // A spec with no values contributes nothing to the cartesian product and collapses it to
        // zero combinations, which would leave `best_idx` indexing an empty vector.
        if let Some(empty) = param_grid.iter().find(|spec| spec.values.is_empty()) {
            return Err(flow_like_types::anyhow!(
                "Parameter `{}` in the Parameter Grid has no values to try. Give it at least one value or remove it.",
                empty.name
            ));
        }

        // Catches the grid left over from a previously selected Model Type, which would otherwise
        // score the same configuration repeatedly and report it as a tuned result.
        let accepted = known_params(&model_type);
        let unknown: Vec<&str> = param_grid
            .iter()
            .map(|spec| spec.name.as_str())
            .filter(|name| !accepted.contains(name))
            .collect();
        if !unknown.is_empty() {
            return Err(flow_like_types::anyhow!(
                "{model_type} does not use these Parameter Grid entries: {}. {}",
                unknown.join(", "),
                if accepted.is_empty() {
                    format!(
                        "{model_type} has no tunable parameters here, so clear the Parameter Grid."
                    )
                } else {
                    format!("It accepts: {}.", accepted.join(", "))
                }
            ));
        }

        // Generate parameter combinations
        let param_combinations = generate_param_combinations(&param_grid);
        context.log_message(
            &format!(
                "Testing {} parameter combinations",
                param_combinations.len()
            ),
            LogLevel::Info,
        );

        let mut all_results = Vec::with_capacity(param_combinations.len());
        let mut best_score = f64::NEG_INFINITY;
        let mut best_idx = 0;

        // Shuffle indices for CV
        let n_samples = records.nsamples();
        // With fewer rows than folds, fold_size is 0: every fold but the last scores an empty
        // validation set as 0.0 and the last one trains on an empty split.
        if n_samples < cv_folds {
            return Err(flow_like_types::anyhow!(
                "Cannot run {cv_folds}-fold cross-validation on {n_samples} rows. Reduce CV Folds or supply more training data."
            ));
        }
        let mut indices: Vec<usize> = (0..n_samples).collect();
        {
            let mut rng = rand::rng();
            indices.shuffle(&mut rng);
        }

        // Calculate fold sizes
        let fold_size = n_samples / cv_folds;

        for (combo_idx, params) in param_combinations.iter().enumerate() {
            let combo_start = Instant::now();
            let mut fold_scores = Vec::with_capacity(cv_folds);

            // K-fold cross validation
            for fold in 0..cv_folds {
                let val_start = fold * fold_size;
                let val_end = if fold == cv_folds - 1 {
                    n_samples
                } else {
                    val_start + fold_size
                };

                // Split indices
                let val_indices: Vec<usize> = indices[val_start..val_end].to_vec();
                let train_indices: Vec<usize> = indices
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i < val_start || *i >= val_end)
                    .map(|(_, &idx)| idx)
                    .collect();

                // Create train/val datasets
                let train_records = records.records().select(ndarray::Axis(0), &train_indices);
                let train_targets: ndarray::Array1<usize> = train_indices
                    .iter()
                    .map(|&i| records.targets()[i])
                    .collect();
                let train_ds = DatasetBase::from(train_records).with_targets(train_targets);

                let val_records = records.records().select(ndarray::Axis(0), &val_indices);
                let val_targets: Vec<usize> =
                    val_indices.iter().map(|&i| records.targets()[i]).collect();

                let score =
                    fit_and_score(&model_type, params, &train_ds, val_records, &val_targets)?;
                fold_scores.push(score);
            }

            let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
            let variance = fold_scores
                .iter()
                .map(|s| (s - mean_score).powi(2))
                .sum::<f64>()
                / fold_scores.len() as f64;
            let std_score = variance.sqrt();

            let entry = GridSearchEntry {
                params: params.clone(),
                mean_score,
                std_score,
                fold_scores,
                train_time_secs: combo_start.elapsed().as_secs_f64(),
            };

            if mean_score > best_score {
                best_score = mean_score;
                best_idx = combo_idx;
            }

            context.log_message(
                &format!(
                    "Combo {}/{}: score={:.4} ± {:.4}",
                    combo_idx + 1,
                    param_combinations.len(),
                    mean_score,
                    std_score
                ),
                LogLevel::Debug,
            );

            all_results.push(entry);
        }

        let best_params = param_combinations[best_idx].clone();

        // Train final model with best params on full data
        let final_model = fit_final_model(&model_type, &best_params, &records, classes.clone())?;

        let result = GridSearchResult {
            results: all_results,
            best_index: best_idx,
            best_params,
            best_score,
            total_time_secs: start_time.elapsed().as_secs_f64(),
            n_combinations: param_combinations.len(),
            n_folds: cv_folds,
        };

        context.log_message(
            &format!(
                "Grid Search complete: best score={:.4} in {:.2}s",
                best_score, result.total_time_secs
            ),
            LogLevel::Info,
        );

        let node_model = NodeMLModel::new(context, final_model).await;
        context.set_pin_value("results", json!(result)).await?;
        context
            .set_pin_value("best_model", json!(node_model))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }

    #[cfg(feature = "execute")]
    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let model_type: String = node
            .get_pin_by_name("model_type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let source_pin: String = node
            .get_pin_by_name("source")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        // The pin is seeded once with the grid for whichever model type was selected at the time.
        // It is deliberately never re-seeded here: rewriting a pin's default on every board parse
        // would clobber a hand-edited grid, and dropping and re-adding the pin would break any
        // connection into it. Instead `run` substitutes the model's default grid when the pin is
        // left empty, so switching Model Type still does the right thing.
        if node.get_pin_by_name("param_grid").is_none() {
            node.add_input_pin(
                "param_grid",
                "Parameter Grid",
                "Parameters to search over. Leave empty to use the default grid for the selected Model Type.",
                VariableType::Struct,
            )
            .set_schema::<Vec<ParameterSpec>>()
            .set_default_value(Some(default_param_grid(&model_type)));
        }

        // Add database pins if needed
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
                    "Column containing feature vectors",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("targets").is_none() {
                node.add_input_pin(
                    "targets",
                    "Target Col",
                    "Column containing target labels",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}

/// A classification dataset in the shape every tuner branch consumes.
#[cfg(feature = "execute")]
type ClassificationDataset = DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>;

/// Number of distinct class ids in a target array.
#[cfg(feature = "execute")]
fn distinct_class_count(targets: &ndarray::Array1<usize>) -> usize {
    targets.iter().copied().collect::<HashSet<usize>>().len()
}

/// Decision tree parameters from a grid combination, falling back to linfa's defaults.
#[cfg(feature = "execute")]
fn tree_params_from(
    params: &HashMap<String, Value>,
) -> linfa_trees::DecisionTreeParams<f64, usize> {
    let mut tree_params = LinfaDecisionTree::params();
    if let Some(depth) = params.get("max_depth").and_then(|v| v.as_i64()) {
        tree_params = tree_params.max_depth(Some(depth as usize));
    }
    let min_weight = params
        .get("min_weight_split")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0) as f32;
    tree_params.min_weight_split(min_weight)
}

/// Fits one model family on `train_ds` and scores it against the held-out fold.
///
/// `model_type` is an [`MLModel::kind`] string; every branch here must have a matching branch in
/// [`fit_final_model`], otherwise a combination could win the search and then fail to retrain.
#[cfg(feature = "execute")]
fn fit_and_score(
    model_type: &str,
    params: &HashMap<String, Value>,
    train_ds: &ClassificationDataset,
    val_records: ndarray::Array2<f64>,
    val_targets: &[usize],
) -> Result<f64> {
    let predictions: ndarray::Array1<usize> = match model_type {
        "GaussianNaiveBayes" => {
            let model = GaussianNb::params().fit(train_ds)?;
            model.predict(&DatasetBase::from(val_records))
        }
        "DecisionTree" => {
            let model = tree_params_from(params).fit(train_ds)?;
            model.predict(&DatasetBase::from(val_records))
        }
        "LogisticRegression" => {
            let alpha = params.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0);
            if distinct_class_count(train_ds.targets()) > 2 {
                let model = MultiLogisticRegression::<f64>::default()
                    .alpha(alpha)
                    .fit(train_ds)
                    .map_err(|err| {
                        flow_like_types::anyhow!(
                            "Multinomial Logistic Regression fit failed: {err}"
                        )
                    })?;
                model.predict(&val_records)
            } else {
                let model = LogisticRegression::<f64>::default()
                    .alpha(alpha)
                    .fit(train_ds)
                    .map_err(|err| {
                        flow_like_types::anyhow!("Logistic Regression fit failed: {err}")
                    })?;
                model.predict(&val_records)
            }
        }
        "RandomForest" => {
            let model = forest_params_from(params, train_ds.records().ncols())
                .fit(train_ds)
                .map_err(|err| flow_like_types::anyhow!("Random Forest fit failed: {err}"))?;
            model.predict(&val_records)
        }
        "SVMMultiClass" => {
            let svm_models = fit_one_vs_all_svm(train_ds)?;
            MultiClassModel::from_iter(svm_models).predict(&DatasetBase::from(val_records))
        }
        other => {
            return Err(flow_like_types::anyhow!(
                "Unknown model type: `{other}`. Supported: {}",
                TUNABLE_MODELS.join(", ")
            ));
        }
    };
    Ok(compute_accuracy(&predictions, val_targets))
}

/// Random forest parameters from a grid combination.
///
/// `feature_proportion` must be greater than zero, so the standard sqrt(p) default is materialised
/// here once the feature count is known.
#[cfg(feature = "execute")]
fn forest_params_from(
    params: &HashMap<String, Value>,
    n_features: usize,
) -> linfa_ensemble::EnsembleLearnerParams<
    linfa_trees::DecisionTreeParams<f64, usize>,
    Xoshiro256Plus,
> {
    let ensemble_size = params
        .get("ensemble_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(100)
        .max(1) as usize;
    let bootstrap = params
        .get("bootstrap_proportion")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1.0)
        .unwrap_or(0.7);
    let feature_proportion = params
        .get("feature_proportion")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1.0)
        .unwrap_or_else(|| {
            ((n_features.max(1) as f64).sqrt() / n_features.max(1) as f64)
                .clamp(f64::MIN_POSITIVE, 1.0)
        });

    // Tuning must be reproducible across runs, so the forest RNG is seeded rather than threaded.
    RandomForestParams::new_fixed_rng(tree_params_from(params), Xoshiro256Plus::seed_from_u64(42))
        .ensemble_size(ensemble_size)
        .bootstrap_proportion(bootstrap)
        .feature_proportion(feature_proportion)
}

/// Fits the one-vs-all SVM ensemble the SVM classifier node produces.
#[cfg(feature = "execute")]
fn fit_one_vs_all_svm(dataset: &ClassificationDataset) -> Result<Vec<(usize, Svm<f64, Pr>)>> {
    let params = Svm::<_, Pr>::params().gaussian_kernel(GAUSSIAN_KERNEL_EPS);
    dataset
        .one_vs_all()?
        .into_iter()
        .map(|(label, binary)| {
            params
                .fit(&binary)
                .map(|model| (label, model))
                .map_err(|err| flow_like_types::anyhow!("SVM fit failed for class {label}: {err}"))
        })
        .collect()
}

/// Retrains the winning configuration on the full dataset.
#[cfg(feature = "execute")]
fn fit_final_model(
    model_type: &str,
    params: &HashMap<String, Value>,
    records: &ClassificationDataset,
    classes: Option<HashMap<usize, String>>,
) -> Result<MLModel> {
    Ok(match model_type {
        "GaussianNaiveBayes" => MLModel::GaussianNaiveBayes(ModelWithMeta {
            model: GaussianNb::params().fit(records)?,
            classes,
        }),
        "DecisionTree" => MLModel::DecisionTree(ModelWithMeta {
            model: tree_params_from(params).fit(records)?,
            classes,
        }),
        "LogisticRegression" => {
            let alpha = params.get("alpha").and_then(|v| v.as_f64()).unwrap_or(1.0);
            if distinct_class_count(records.targets()) > 2 {
                MLModel::MultinomialLogisticRegression(ModelWithMeta {
                    model: MultiLogisticRegression::<f64>::default()
                        .alpha(alpha)
                        .fit(records)
                        .map_err(|err| {
                            flow_like_types::anyhow!(
                                "Multinomial Logistic Regression fit failed: {err}"
                            )
                        })?,
                    classes,
                })
            } else {
                MLModel::LogisticRegression(ModelWithMeta {
                    model: LogisticRegression::<f64>::default()
                        .alpha(alpha)
                        .fit(records)
                        .map_err(|err| {
                            flow_like_types::anyhow!("Logistic Regression fit failed: {err}")
                        })?,
                    classes,
                })
            }
        }
        "RandomForest" => MLModel::RandomForest(ModelWithMeta {
            model: PersistedEnsemble(
                forest_params_from(params, records.records().ncols())
                    .fit(records)
                    .map_err(|err| flow_like_types::anyhow!("Random Forest fit failed: {err}"))?,
            ),
            classes,
        }),
        "SVMMultiClass" => MLModel::SVMMultiClass(ModelWithMeta {
            model: fit_one_vs_all_svm(records)?,
            classes,
        }),
        other => {
            return Err(flow_like_types::anyhow!(
                "Unknown model type: `{other}`. Supported: {}",
                TUNABLE_MODELS.join(", ")
            ));
        }
    })
}

#[cfg(feature = "execute")]
fn generate_param_combinations(grid: &[ParameterSpec]) -> Vec<HashMap<String, Value>> {
    if grid.is_empty() {
        return vec![HashMap::new()];
    }

    let mut result = vec![HashMap::new()];

    for spec in grid {
        let mut new_result = Vec::with_capacity(result.len() * spec.values.len());
        for existing in &result {
            for value in &spec.values {
                let mut combo = existing.clone();
                combo.insert(spec.name.clone(), value.clone());
                new_result.push(combo);
            }
        }
        result = new_result;
    }

    result
}

#[cfg(feature = "execute")]
fn compute_accuracy(predictions: &ndarray::Array1<usize>, targets: &[usize]) -> f64 {
    if predictions.len() != targets.len() || predictions.is_empty() {
        return 0.0;
    }
    let correct = predictions
        .iter()
        .zip(targets.iter())
        .filter(|(p, t)| p == t)
        .count();
    correct as f64 / predictions.len() as f64
}
