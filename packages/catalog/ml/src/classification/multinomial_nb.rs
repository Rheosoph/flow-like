//! Node for Fitting Multinomial Naive Bayes Classifier
//!
//! This node loads a dataset, transforms it into a classification dataset,
//! and fits a Multinomial Naive Bayes model using the [`linfa_bayes`] crate.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array1_target,
    values_to_array2_f64,
};
use flow_like::flow::board::Board;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
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
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_bayes::MultinomialNb;
#[cfg(feature = "execute")]
use ndarray::Array2;
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Rejects feature matrices that multinomial Naive Bayes cannot model.
///
/// Smoothed counts are turned into log probabilities directly, so a negative or non-finite cell
/// becomes a NaN log probability. That NaN is never reported: it survives the fit and only surfaces
/// at prediction time, where linfa's `argmax().unwrap()` panics on the undefined ordering.
#[cfg(feature = "execute")]
fn ensure_count_features(features: &Array2<f64>, column: &str) -> Result<()> {
    if features.ncols() == 0 {
        return Err(anyhow!(
            "Column `{column}` holds zero-width feature vectors, so there is nothing to learn from"
        ));
    }
    if let Some(((row, col), value)) = features
        .indexed_iter()
        .find(|(_, value)| !value.is_finite() || **value < 0.0)
    {
        return Err(anyhow!(
            "Multinomial Naive Bayes models counts and requires non-negative, finite features, but row {row} feature {col} of column `{column}` is {value}. Feed it raw counts or TF-IDF weights (see the Fit TF-IDF Vectorizer node) instead of centered or standardized vectors."
        ));
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct FitMultinomialNaiveBayesNode {}

impl FitMultinomialNaiveBayesNode {
    pub fn new() -> Self {
        FitMultinomialNaiveBayesNode {}
    }
}

#[async_trait]
impl NodeLogic for FitMultinomialNaiveBayesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_multinomial_naive_bayes",
            "Train Classifier (Multinomial Naive Bayes)",
            "Fit/Train a Multinomial Naive Bayes classifier, the standard baseline for text and other count data. Features must be non-negative counts or TF-IDF weights, which is what the Fit TF-IDF Vectorizer node produces. Native multi-class support and a single pass over the data.",
            "AI/ML/Classification",
        );
        node.set_flowscript_name("ml", "fitMultinomialNaiveBayes");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(9) // Single pass over the data, no iterative optimization
                .set_governance(6)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins Multinomial Naive Bayes training",
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
            "alpha",
            "Alpha",
            "Additive (Laplace/Lidstone) smoothing added to every feature count. 1.0 is the usual choice; smaller values trust the training counts more, and 0 disables smoothing so any term unseen in a class makes that class impossible.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 100.)).build())
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
            "Thread-safe handle to the trained Multinomial Naive Bayes classifier",
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
        let alpha: f64 = context.evaluate_pin("alpha").await?;

        if !alpha.is_finite() || alpha < 0.0 {
            return Err(anyhow!(
                "Alpha is a smoothing amount and must be finite and non-negative, got {alpha}"
            ));
        }
        if alpha == 0.0 {
            context.log_message(
                "Alpha is 0, so any feature that never occurs in a class yields a -inf log probability for it",
                LogLevel::Warn,
            );
        }

        let t0 = std::time::Instant::now();
        let (ds, classes) = match source.as_str() {
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
                if records.is_empty() {
                    return Err(anyhow!("No records to train on"));
                }
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );

                let train_array = values_to_array2_f64(&records, &records_col)?;
                ensure_count_features(&train_array, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records, &targets_col)?;

                let distinct: HashSet<usize> = target_array.iter().copied().collect();
                if distinct.len() < 2 {
                    return Err(anyhow!(
                        "Target col `{}` holds {} distinct class(es); a classifier needs at least 2",
                        targets_col,
                        distinct.len()
                    ));
                }
                (
                    DatasetBase::from(train_array).with_targets(target_array),
                    classes,
                )
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let t0 = std::time::Instant::now();
        // linfa 0.8.1 leaves a `dbg!` of the per-class histogram in this fit path, so one block per
        // class is printed to stderr while training.
        let params = MultinomialNb::params().alpha(alpha);
        let nb_model = params.fit(&ds)?;
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        let model = MLModel::MultinomialNaiveBayes(ModelWithMeta {
            model: nb_model,
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
                    "Column Containing the Count or TF-IDF Vectors to Train on",
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
        }
    }
}
