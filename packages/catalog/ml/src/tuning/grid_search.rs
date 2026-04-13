//! Grid Search Hyperparameter Tuning
//!
//! Exhaustive search over parameter grid with cross-validation.

use crate::ml::{GridSearchEntry, GridSearchResult, NodeMLModel, ParameterSpec};
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
use linfa::dataset::Records;
#[cfg(feature = "execute")]
use linfa::traits::{Fit, Predict};
#[cfg(feature = "execute")]
use linfa_bayes::GaussianNb;
#[cfg(feature = "execute")]
use linfa_trees::DecisionTree as LinfaDecisionTree;
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
            "Exhaustive search over parameter combinations with cross-validation. Returns the best parameters found.",
            "AI/ML/Tuning",
        );
        node.add_icon("/flow/icons/chart-network.svg");

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
                .set_valid_values(vec!["NaiveBayes".to_string(), "DecisionTree".to_string()])
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

        let cv_folds = cv_folds as usize;
        if cv_folds < 2 {
            return Err(flow_like_types::anyhow!("CV folds must be at least 2"));
        }

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

                // Train and evaluate
                let score = match model_type.as_str() {
                    "NaiveBayes" => {
                        let model = GaussianNb::params().fit(&train_ds)?;
                        let val_ds = DatasetBase::from(val_records);
                        let predictions = model.predict(&val_ds);
                        compute_accuracy(&predictions, &val_targets)
                    }
                    "DecisionTree" => {
                        let max_depth = params
                            .get("max_depth")
                            .and_then(|v| v.as_i64())
                            .map(|v| v as usize);
                        let min_weight = params
                            .get("min_weight_split")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(1.0) as f32;

                        let mut tree_params = LinfaDecisionTree::params();
                        if let Some(depth) = max_depth {
                            tree_params = tree_params.max_depth(Some(depth));
                        }
                        tree_params = tree_params.min_weight_split(min_weight);

                        let model = tree_params.fit(&train_ds)?;
                        let val_ds = DatasetBase::from(val_records);
                        let predictions = model.predict(&val_ds);
                        compute_accuracy(&predictions, &val_targets)
                    }
                    _ => {
                        return Err(flow_like_types::anyhow!(
                            "Unknown model type: {}",
                            model_type
                        ));
                    }
                };

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
        let final_model = match model_type.as_str() {
            "NaiveBayes" => {
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
                    .map(|v| v as usize);
                let min_weight = best_params
                    .get("min_weight_split")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0) as f32;

                let mut tree_params = LinfaDecisionTree::params();
                if let Some(depth) = max_depth {
                    tree_params = tree_params.max_depth(Some(depth));
                }
                tree_params = tree_params.min_weight_split(min_weight);

                let model = tree_params.fit(&records)?;
                MLModel::DecisionTree(ModelWithMeta {
                    model,
                    classes: classes.clone(),
                })
            }
            _ => return Err(flow_like_types::anyhow!("Unknown model type")),
        };

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

        // Add parameter grid pin based on model type
        if node.get_pin_by_name("param_grid").is_none() {
            let default_grid = match model_type.as_str() {
                "DecisionTree" => json!([
                    {"name": "max_depth", "values": [5, 10, 15, 20]},
                    {"name": "min_weight_split", "values": [1.0, 2.0, 5.0]}
                ]),
                "NaiveBayes" => json!([]),
                _ => json!([]),
            };

            node.add_input_pin(
                "param_grid",
                "Parameter Grid",
                "Parameters to search over",
                VariableType::Struct,
            )
            .set_schema::<Vec<ParameterSpec>>()
            .set_default_value(Some(default_grid));
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
        }
    }
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
