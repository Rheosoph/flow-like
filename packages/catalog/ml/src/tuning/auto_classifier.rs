//! AutoML Classifier
//!
//! Automatically tries multiple classification algorithms and returns the best one.
//! Simple AutoML that compares models with cross-validation.

use crate::ml::{AutoMLEntry, AutoMLResult, NodeMLModel};
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array1_target,
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
use linfa_svm::Svm;
#[cfg(feature = "execute")]
use linfa_trees::DecisionTree as LinfaDecisionTree;
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

#[cfg(feature = "execute")]
const GAUSSIAN_KERNEL_EPS: f64 = 30.0;

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
            "Automatically finds the best classification model. Tries Naive Bayes, Decision Tree, and SVM with cross-validation.",
            "AI/ML/Tuning",
        );
        node.add_icon("/flow/icons/chart-network.svg");

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
            "Optimization metric",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["accuracy".to_string()])
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
        let source: String = context.evaluate_pin("source").await?;

        let cv_folds = cv_folds as usize;
        if cv_folds < 2 {
            return Err(flow_like_types::anyhow!("CV folds must be at least 2"));
        }

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
                let score = compute_accuracy(&predictions, &val_targets);
                fold_scores.push(score);
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
                    let score = compute_accuracy(&predictions, &val_targets);
                    fold_scores.push(score);
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

        // 3. SVM (optional, slower)
        if include_svm {
            let svm_result = {
                let model_start = Instant::now();
                let mut fold_scores = Vec::with_capacity(cv_folds);

                for fold in 0..cv_folds {
                    let (train_ds, val_records, val_targets) =
                        create_fold_split(&records, &indices, fold, fold_size, cv_folds);

                    // Train OvA SVM
                    let params = Svm::<_, Pr>::params().gaussian_kernel(GAUSSIAN_KERNEL_EPS);
                    let svm_models: Vec<(usize, Svm<f64, Pr>)> = train_ds
                        .one_vs_all()?
                        .into_iter()
                        .map(|(l, x)| (l, params.fit(&x).unwrap()))
                        .collect();

                    let mult_class = MultiClassModel::from_iter(svm_models);
                    let val_ds = DatasetBase::from(val_records);
                    let predictions = mult_class.predict(&val_ds);
                    let score = compute_accuracy(&predictions, &val_targets);
                    fold_scores.push(score);
                }

                let mean_score = fold_scores.iter().sum::<f64>() / fold_scores.len() as f64;
                context.log_message(&format!("SVM: CV score={:.4}", mean_score), LogLevel::Info);

                AutoMLEntry {
                    model_type: "SVMMultiClass".to_string(),
                    best_params: HashMap::new(),
                    cv_score: mean_score,
                    train_time_secs: model_start.elapsed().as_secs_f64(),
                    rank: 0,
                }
            };
            leaderboard.push(svm_result);
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
            "SVMMultiClass" => {
                let params = Svm::<_, Pr>::params().gaussian_kernel(GAUSSIAN_KERNEL_EPS);
                let svm_models: Vec<(usize, Svm<f64, Pr>)> = records
                    .one_vs_all()?
                    .into_iter()
                    .map(|(l, x)| (l, params.fit(&x).unwrap()))
                    .collect();
                MLModel::SVMMultiClass(ModelWithMeta {
                    model: svm_models,
                    classes: classes.clone(),
                })
            }
            _ => return Err(flow_like_types::anyhow!("Unknown best model type")),
        };

        let result = AutoMLResult {
            leaderboard,
            best_model_index: 0,
            total_models_tried: if include_svm { 3 } else { 2 },
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
        }
    }
}

#[cfg(feature = "execute")]
fn create_fold_split(
    records: &DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
    indices: &[usize],
    fold: usize,
    fold_size: usize,
    cv_folds: usize,
) -> (
    DatasetBase<ndarray::Array2<f64>, ndarray::Array1<usize>>,
    ndarray::Array2<f64>,
    Vec<usize>,
) {
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
