//! AutoML Classifier
//!
//! Automatically tries multiple classification algorithms and returns the best one.
//! Simple AutoML that compares models with cross-validation.

use crate::ml::{AutoMLEntry, AutoMLResult, NodeMLModel};
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

/// Kernel width shared with the standalone SVM node so the tuned SVM matches what that node trains.
#[cfg(feature = "execute")]
const GAUSSIAN_KERNEL_EPS: f64 = 30.0;

/// Seed for the ensemble RNG, so re-running a flow produces the same leaderboard.
#[cfg(feature = "execute")]
const FOREST_SEED: u64 = 42;

/// Scores predictions under the selected optimization metric.
///
/// `accuracy` is the share of correct rows; `macro_f1` averages the per-class F1 with equal weight
/// per class, which is what you want when the classes are imbalanced and accuracy would be carried
/// by the majority class alone.
#[cfg(feature = "execute")]
fn score_predictions(
    metric: &str,
    predictions: &ndarray::Array1<usize>,
    targets: &[usize],
) -> Result<f64> {
    match metric {
        "accuracy" => Ok(compute_accuracy(predictions, targets)),
        "macro_f1" => Ok(compute_macro_f1(predictions, targets)),
        other => Err(flow_like_types::anyhow!(
            "Unknown metric `{other}`. Supported: accuracy, macro_f1"
        )),
    }
}

/// Trees grown per Random Forest candidate. Kept modest because it is paid once per CV fold.
#[cfg(feature = "execute")]
const FOREST_ENSEMBLE_SIZE: usize = 100;

/// Number of distinct class ids in a target array.
#[cfg(feature = "execute")]
fn distinct_class_count(targets: &ndarray::Array1<usize>) -> usize {
    targets.iter().copied().collect::<HashSet<usize>>().len()
}

/// Fits a logistic regression of the right arity and predicts the held-out fold.
#[cfg(feature = "execute")]
fn fit_logistic_predict(
    train_ds: &DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
    val_records: &ndarray::Array2<f64>,
    alpha: f64,
    is_multinomial: bool,
) -> Result<ndarray::Array1<usize>> {
    if is_multinomial {
        let model = MultiLogisticRegression::<f64>::default()
            .alpha(alpha)
            .fit(train_ds)
            .map_err(|err| {
                flow_like_types::anyhow!("Multinomial Logistic Regression fit failed: {err}")
            })?;
        Ok(model.predict(val_records))
    } else {
        let model = LogisticRegression::<f64>::default()
            .alpha(alpha)
            .fit(train_ds)
            .map_err(|err| flow_like_types::anyhow!("Logistic Regression fit failed: {err}"))?;
        Ok(model.predict(val_records))
    }
}

/// Random forest parameters used for every forest candidate.
///
/// `feature_proportion` must be greater than zero, so the standard sqrt(p) default is materialised
/// here once the feature count is known. The RNG is seeded so the leaderboard is reproducible.
#[cfg(feature = "execute")]
fn forest_params(
    n_features: usize,
) -> linfa_ensemble::EnsembleLearnerParams<
    linfa_trees::DecisionTreeParams<f64, usize>,
    Xoshiro256Plus,
> {
    let features = n_features.max(1);
    let proportion = ((features as f64).sqrt() / features as f64).clamp(f64::MIN_POSITIVE, 1.0);
    RandomForestParams::new_fixed_rng(
        LinfaDecisionTree::params().max_depth(Some(10)),
        Xoshiro256Plus::seed_from_u64(FOREST_SEED),
    )
    .ensemble_size(FOREST_ENSEMBLE_SIZE)
    .bootstrap_proportion(0.7)
    .feature_proportion(proportion)
}

/// Fits the one-vs-all SVM ensemble, surfacing per-class failures instead of panicking.
#[cfg(feature = "execute")]
fn fit_one_vs_all_svm(
    dataset: &DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
) -> Result<Vec<(usize, Svm<f64, Pr>)>> {
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

/// Unweighted mean of the per-class F1 scores.
#[cfg(feature = "execute")]
fn compute_macro_f1(predictions: &ndarray::Array1<usize>, targets: &[usize]) -> f64 {
    if predictions.len() != targets.len() || predictions.is_empty() {
        return 0.0;
    }

    let classes: HashSet<usize> = targets
        .iter()
        .copied()
        .chain(predictions.iter().copied())
        .collect();
    if classes.is_empty() {
        return 0.0;
    }

    let mut total = 0.0;
    for class in &classes {
        let mut true_positive = 0.0;
        let mut false_positive = 0.0;
        let mut false_negative = 0.0;
        for (predicted, actual) in predictions.iter().zip(targets.iter()) {
            match (predicted == class, actual == class) {
                (true, true) => true_positive += 1.0,
                (true, false) => false_positive += 1.0,
                (false, true) => false_negative += 1.0,
                (false, false) => {}
            }
        }
        // A class with no predictions and no instances contributes 0 rather than NaN.
        let denominator = 2.0 * true_positive + false_positive + false_negative;
        if denominator > 0.0 {
            total += 2.0 * true_positive / denominator;
        }
    }
    total / classes.len() as f64
}

#[crate::register_node]
#[derive(Default)]
pub struct AutoClassifierNode {}

impl AutoClassifierNode {
    pub fn new() -> Self {
        AutoClassifierNode {}
    }
}

#[async_trait]
impl NodeLogic for AutoClassifierNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_ml_tuning_auto_classifier",
            "Auto Classifier",
            "Automatically finds the best classification model. Cross-validates Naive Bayes, Decision Tree, Logistic Regression, Random Forest and SVM, then retrains the winner on the full dataset. The reported Best Model Type can be fed straight into Grid Search to tune it further.",
            "AI/ML/Tuning",
        );
        node.set_flowscript_name("ml", "autoClassifier");
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_version(2);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(4)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(4) // Trains multiple models
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "cv_folds",
            "CV Folds",
            "Number of cross-validation folds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "metric",
            "Metric",
            "Metric the leaderboard is ranked by. Accuracy is the share of correct rows; Macro F1 averages per-class F1 with equal weight per class, which is the right choice when the classes are imbalanced.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["accuracy".to_string(), "macro_f1".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("accuracy")));

        node.add_input_pin(
            "include_svm",
            "Include SVM",
            "Include SVM in comparison (slower but often more accurate)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_logistic",
            "Include Logistic Regression",
            "Include Logistic Regression. Fast, and the only candidate that yields calibrated probabilities, but it expects scaled features — fit a Feature Scaler first for a fair comparison.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_random_forest",
            "Include Random Forest",
            "Include Random Forest. Usually the strongest candidate here, at the cost of training one tree per ensemble member on every fold.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "source",
            "Data Source",
            "Data source type",
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
            "Activated when AutoML completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Complete AutoML results with leaderboard",
            VariableType::Struct,
        )
        .set_schema::<AutoMLResult>();

        node.add_output_pin(
            "best_model",
            "Best Model",
            "The best model trained on full data",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "best_model_type",
            "Best Model Type",
            "Name of the best algorithm",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use std::time::Instant;

        context.deactivate_exec_pin("exec_out").await?;

        let cv_folds: i64 = context.evaluate_pin("cv_folds").await?;
        let metric: String = context.evaluate_pin("metric").await?;
        let include_svm: bool = context.evaluate_pin("include_svm").await?;
        let include_logistic: bool = context.evaluate_pin("include_logistic").await?;
        let include_random_forest: bool = context.evaluate_pin("include_random_forest").await?;
        let source: String = context.evaluate_pin("source").await?;

        // Checked before the cast: a negative value would wrap to usize::MAX and sail past the
        // guard, then blow up in Vec::with_capacity.
        if cv_folds < 2 {
            return Err(flow_like_types::anyhow!(
                "CV folds must be at least 2, got {cv_folds}"
            ));
        }
        let cv_folds = cv_folds as usize;
        // Validate up front rather than after paying for a full sweep.
        score_predictions(&metric, &ndarray::Array1::from(vec![0usize]), &[0usize])?;

        let start_time = Instant::now();

        // Load data
        let (records, classes) = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let targets_col: String = context.evaluate_pin("targets").await?;

                let records_data = {
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

                let train_array = values_to_array2_f64(&records_data, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records_data, &targets_col)?;
                (
                    DatasetBase::from(train_array).with_targets(target_array),
                    classes,
                )
            }
            _ => return Err(flow_like_types::anyhow!("Datasource Not Implemented!")),
        };

        context.log_message(
            &format!(
                "AutoML: {} samples, {} folds, metric={}",
                records.nsamples(),
                cv_folds,
                metric
            ),
            LogLevel::Info,
        );

        // Prepare CV splits
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
        let fold_size = n_samples / cv_folds;

        let mut leaderboard = Vec::new();

        // 1. Naive Bayes (fast baseline)
        let nb_result = {
            let model_start = Instant::now();
            let mut fold_scores = Vec::with_capacity(cv_folds);

            for fold in 0..cv_folds {
                let (train_ds, val_records, val_targets) =
                    create_fold_split(&records, &indices, fold, fold_size, cv_folds);
                let model = GaussianNb::params().fit(&train_ds)?;
                let val_ds = DatasetBase::from(val_records);
                let predictions = model.predict(&val_ds);
                fold_scores.push(score_predictions(&metric, &predictions, &val_targets)?);
            }

            let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
            context.log_message(
                &format!("NaiveBayes: CV score={:.4}", mean_score),
                LogLevel::Info,
            );

            AutoMLEntry {
                model_type: "GaussianNaiveBayes".to_string(),
                best_params: HashMap::new(),
                cv_score: mean_score,
                train_time_secs: model_start.elapsed().as_secs_f64(),
                rank: 0,
            }
        };
        leaderboard.push(nb_result);

        // 2. Decision Tree with varying depths
        let dt_result = {
            let model_start = Instant::now();
            let depths = [5, 10, 15];
            let mut best_score = 0.0;
            let mut best_depth = 10;

            for &depth in &depths {
                let mut fold_scores = Vec::with_capacity(cv_folds);

                for fold in 0..cv_folds {
                    let (train_ds, val_records, val_targets) =
                        create_fold_split(&records, &indices, fold, fold_size, cv_folds);
                    let model = LinfaDecisionTree::params()
                        .max_depth(Some(depth))
                        .fit(&train_ds)?;
                    let val_ds = DatasetBase::from(val_records);
                    let predictions = model.predict(&val_ds);
                    fold_scores.push(score_predictions(&metric, &predictions, &val_targets)?);
                }

                let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
                if mean_score > best_score {
                    best_score = mean_score;
                    best_depth = depth;
                }
            }

            context.log_message(
                &format!(
                    "DecisionTree: CV score={:.4} (depth={})",
                    best_score, best_depth
                ),
                LogLevel::Info,
            );

            let mut params = HashMap::new();
            params.insert("max_depth".to_string(), json!(best_depth));

            AutoMLEntry {
                model_type: "DecisionTree".to_string(),
                best_params: params,
                cv_score: best_score,
                train_time_secs: model_start.elapsed().as_secs_f64(),
                rank: 0,
            }
        };
        leaderboard.push(dt_result);

        let is_multinomial = distinct_class_count(records.targets()) > 2;

        // 3. Logistic Regression across a small regularization sweep
        if include_logistic {
            let model_start = Instant::now();
            let alphas = [0.1, 1.0, 10.0];
            let mut best_score = f64::NEG_INFINITY;
            let mut best_alpha = 1.0;

            for &alpha in &alphas {
                let mut fold_scores = Vec::with_capacity(cv_folds);
                for fold in 0..cv_folds {
                    let (train_ds, val_records, val_targets) =
                        create_fold_split(&records, &indices, fold, fold_size, cv_folds);
                    let predictions =
                        fit_logistic_predict(&train_ds, &val_records, alpha, is_multinomial)?;
                    fold_scores.push(score_predictions(&metric, &predictions, &val_targets)?);
                }
                let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
                if mean_score > best_score {
                    best_score = mean_score;
                    best_alpha = alpha;
                }
            }

            context.log_message(
                &format!("LogisticRegression: CV score={best_score:.4} (alpha={best_alpha})"),
                LogLevel::Info,
            );

            let mut params = HashMap::new();
            params.insert("alpha".to_string(), json!(best_alpha));
            leaderboard.push(AutoMLEntry {
                // Reported as one family regardless of arity: binary vs multinomial is decided
                // from the class count at retrain time, and collapsing them keeps the value
                // directly feedable into Grid Search's Model Type.
                model_type: "LogisticRegression".to_string(),
                best_params: params,
                cv_score: best_score,
                train_time_secs: model_start.elapsed().as_secs_f64(),
                rank: 0,
            });
        }

        // 4. Random Forest (slower, usually the strongest baseline)
        if include_random_forest {
            let model_start = Instant::now();
            let mut fold_scores = Vec::with_capacity(cv_folds);

            for fold in 0..cv_folds {
                let (train_ds, val_records, val_targets) =
                    create_fold_split(&records, &indices, fold, fold_size, cv_folds);
                let model = forest_params(train_ds.records().ncols())
                    .fit(&train_ds)
                    .map_err(|err| flow_like_types::anyhow!("Random Forest fit failed: {err}"))?;
                let predictions: ndarray::Array1<usize> = model.predict(&val_records);
                fold_scores.push(score_predictions(&metric, &predictions, &val_targets)?);
            }

            let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
            context.log_message(
                &format!("RandomForest: CV score={mean_score:.4}"),
                LogLevel::Info,
            );

            let mut params = HashMap::new();
            params.insert("ensemble_size".to_string(), json!(FOREST_ENSEMBLE_SIZE));
            leaderboard.push(AutoMLEntry {
                model_type: "RandomForest".to_string(),
                best_params: params,
                cv_score: mean_score,
                train_time_secs: model_start.elapsed().as_secs_f64(),
                rank: 0,
            });
        }

        // 5. SVM (optional, slower)
        if include_svm {
            let model_start = Instant::now();
            let mut fold_scores = Vec::with_capacity(cv_folds);

            for fold in 0..cv_folds {
                let (train_ds, val_records, val_targets) =
                    create_fold_split(&records, &indices, fold, fold_size, cv_folds);
                let svm_models = fit_one_vs_all_svm(&train_ds)?;
                let mult_class = MultiClassModel::from_iter(svm_models);
                let predictions = mult_class.predict(&DatasetBase::from(val_records));
                fold_scores.push(score_predictions(&metric, &predictions, &val_targets)?);
            }

            let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
            context.log_message(&format!("SVM: CV score={mean_score:.4}"), LogLevel::Info);

            leaderboard.push(AutoMLEntry {
                model_type: "SVMMultiClass".to_string(),
                best_params: HashMap::new(),
                cv_score: mean_score,
                train_time_secs: model_start.elapsed().as_secs_f64(),
                rank: 0,
            });
        }

        // Sort by score descending and assign ranks
        leaderboard.sort_by(|a, b| b.cv_score.partial_cmp(&a.cv_score).unwrap());
        for (i, entry) in leaderboard.iter_mut().enumerate() {
            entry.rank = i + 1;
        }

        let best_model_type = leaderboard[0].model_type.clone();
        let best_params = leaderboard[0].best_params.clone();

        // Train final model on full data
        let final_model = match best_model_type.as_str() {
            "GaussianNaiveBayes" => {
                let model = GaussianNb::params().fit(&records)?;
                MLModel::GaussianNaiveBayes(ModelWithMeta {
                    model,
                    classes: classes.clone(),
                })
            }
            "DecisionTree" => {
                let max_depth = best_params
                    .get("max_depth")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as usize)
                    .unwrap_or(10);
                let model = LinfaDecisionTree::params()
                    .max_depth(Some(max_depth))
                    .fit(&records)?;
                MLModel::DecisionTree(ModelWithMeta {
                    model,
                    classes: classes.clone(),
                })
            }
            "SVMMultiClass" => MLModel::SVMMultiClass(ModelWithMeta {
                model: fit_one_vs_all_svm(&records)?,
                classes: classes.clone(),
            }),
            "LogisticRegression" => {
                let alpha = best_params
                    .get("alpha")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                if is_multinomial {
                    MLModel::MultinomialLogisticRegression(ModelWithMeta {
                        model: MultiLogisticRegression::<f64>::default()
                            .alpha(alpha)
                            .fit(&records)
                            .map_err(|err| {
                                flow_like_types::anyhow!(
                                    "Multinomial Logistic Regression fit failed: {err}"
                                )
                            })?,
                        classes: classes.clone(),
                    })
                } else {
                    MLModel::LogisticRegression(ModelWithMeta {
                        model: LogisticRegression::<f64>::default()
                            .alpha(alpha)
                            .fit(&records)
                            .map_err(|err| {
                                flow_like_types::anyhow!("Logistic Regression fit failed: {err}")
                            })?,
                        classes: classes.clone(),
                    })
                }
            }
            "RandomForest" => MLModel::RandomForest(ModelWithMeta {
                model: PersistedEnsemble(
                    forest_params(records.records().ncols())
                        .fit(&records)
                        .map_err(|err| {
                            flow_like_types::anyhow!("Random Forest fit failed: {err}")
                        })?,
                ),
                classes: classes.clone(),
            }),
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown best model type: `{other}`"
                ));
            }
        };

        let result = AutoMLResult {
            total_models_tried: leaderboard.len(),
            leaderboard,
            best_model_index: 0,
            total_time_secs: start_time.elapsed().as_secs_f64(),
            metric,
        };

        context.log_message(
            &format!(
                "AutoML complete: best={} (score={:.4}) in {:.2}s",
                best_model_type, result.leaderboard[0].cv_score, result.total_time_secs
            ),
            LogLevel::Info,
        );

        let node_model = NodeMLModel::new(context, final_model).await;
        context.set_pin_value("results", json!(result)).await?;
        context
            .set_pin_value("best_model", json!(node_model))
            .await?;
        context
            .set_pin_value("best_model_type", json!(best_model_type))
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

/// Training dataset, validation features and the validation row indices of one CV fold.
#[cfg(feature = "execute")]
type FoldSplit = (
    DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
    ndarray::Array2<f64>,
    Vec<usize>,
);

#[cfg(feature = "execute")]
fn create_fold_split(
    records: &DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
    indices: &[usize],
    fold: usize,
    fold_size: usize,
    cv_folds: usize,
) -> FoldSplit {
    let n_samples = records.nsamples();
    let val_start = fold * fold_size;
    let val_end = if fold == cv_folds - 1 {
        n_samples
    } else {
        val_start + fold_size
    };

    let val_indices: Vec<usize> = indices[val_start..val_end].to_vec();
    let train_indices: Vec<usize> = indices
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < val_start || *i >= val_end)
        .map(|(_, &idx)| idx)
        .collect();

    let train_records = records.records().select(ndarray::Axis(0), &train_indices);
    let train_targets: ndarray::Array1<usize> = train_indices
        .iter()
        .map(|&i| records.targets()[i])
        .collect();
    let train_ds = DatasetBase::from(train_records).with_targets(train_targets);

    let val_records = records.records().select(ndarray::Axis(0), &val_indices);
    let val_targets: Vec<usize> = val_indices.iter().map(|&i| records.targets()[i]).collect();

    (train_ds, val_records, val_targets)
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
