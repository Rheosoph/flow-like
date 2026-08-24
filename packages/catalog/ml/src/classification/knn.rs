//! Node for Fitting a **K-Nearest-Neighbours Classifier**
//!
//! linfa ships nearest-neighbour *indexes* but no KNN estimator, so the fitted model is the
//! training matrix itself (see [`crate::ml::KnnModel`]). "Training" therefore only validates the
//! data and materialises it into the model — there is no optimisation step and nothing to converge.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{
    KnnModel, MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array1_target,
    values_to_array2_f64,
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
use std::collections::HashSet;

/// Largest integer class id that survives the `usize -> f64 -> usize` round trip inside
/// [`KnnModel`]. Class ids above this silently change value at prediction time.
#[cfg(feature = "execute")]
const MAX_EXACT_CLASS_ID: u64 = 1u64 << 53;

#[crate::register_node]
#[derive(Default)]
pub struct FitKnnClassifierNode {}

impl FitKnnClassifierNode {
    pub fn new() -> Self {
        FitKnnClassifierNode {}
    }
}

#[async_trait]
impl NodeLogic for FitKnnClassifierNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_knn_classifier",
            "Train Classifier (K-Nearest Neighbours)",
            "Fit a K-Nearest-Neighbours classifier. Non-parametric and instance based: the fitted model embeds a verbatim copy of the whole training set instead of learned coefficients, so every training row (and any personal data in it) travels with the model, is written into every saved model file and can be reconstructed by anyone holding it. Treat the model with the same care as the source table.",
            "AI/ML/Classification",
        );
        node.set_flowscript_name("ml", "fitKnnClassifier");
        node.add_icon("/flow/icons/chart-network.svg");

        // Deliberately far below the other classifiers: the model is the raw training data, not a
        // set of learned parameters, so it leaks records and cannot be audited as a rule set.
        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(4)
                .set_governance(2)
                .set_reliability(7)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins KNN training",
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
            "k",
            "Neighbours (k)",
            "How many nearest training rows vote on each prediction. Must be at least 1 and cannot exceed the number of training rows. Larger values smooth the decision boundary.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 1000.)).build())
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "distance_weighted",
            "Distance Weighted",
            "Weight each neighbour by the inverse of its distance instead of counting every neighbour equally. Helps when k is large or classes overlap.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the training set has been validated and embedded",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained KNN classifier. Contains a full copy of the training set.",
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
        let k: i64 = context.evaluate_pin("k").await?;
        let distance_weighted: bool = context.evaluate_pin("distance_weighted").await?;

        if k < 1 {
            return Err(anyhow!(
                "KNN requires k >= 1, got {k}. Set the Neighbours (k) pin to a positive value."
            ));
        }
        let k = k as usize;

        let t0 = std::time::Instant::now();
        let (train_array, target_array, classes) = match source.as_str() {
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

                if records.is_empty() {
                    return Err(anyhow!(
                        "KNN needs at least {k} training rows, but column `{records_col}` returned none."
                    ));
                }

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (target_array, classes) = values_to_array1_target(&records, &targets_col)?;
                (train_array, target_array, classes)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        let n_rows = train_array.nrows();
        let n_features = train_array.ncols();
        if n_features == 0 {
            return Err(anyhow!(
                "KNN training rows have no features. The train column must hold non-empty numeric vectors."
            ));
        }
        if n_rows < k {
            return Err(anyhow!(
                "KNN was asked for k = {k} neighbours but only {n_rows} training rows are available. Lower k or supply more data."
            ));
        }
        if target_array.len() != n_rows {
            return Err(anyhow!(
                "KNN feature/target mismatch: {n_rows} feature rows vs {} targets.",
                target_array.len()
            ));
        }

        // Logical iteration order of an `Array2` is row-major, which is exactly the layout
        // `KnnModel::features` is queried with.
        let features: Vec<f64> = train_array.iter().copied().collect();
        if let Some(offset) = features.iter().position(|value| !value.is_finite()) {
            // Non-finite features poison the distance sort, which falls back to `Ordering::Equal`
            // and would silently return arbitrary neighbours.
            return Err(anyhow!(
                "KNN training row {} column {} is not a finite number. Clean or impute the data before training.",
                offset / n_features,
                offset % n_features
            ));
        }

        for (row, class) in target_array.iter().enumerate() {
            if *class as u64 > MAX_EXACT_CLASS_ID {
                return Err(anyhow!(
                    "KNN class id {} in training row {row} is too large to be represented exactly. Map the target column to small class ids first.",
                    class
                ));
            }
        }
        let targets: Vec<f64> = target_array.iter().map(|class| *class as f64).collect();

        let distinct_classes: HashSet<usize> = target_array.iter().copied().collect();
        if distinct_classes.len() < 2 {
            context.log_message(
                "KNN training set contains a single class; every prediction will return that class.",
                LogLevel::Warn,
            );
        }
        if !distance_weighted && k.is_multiple_of(2) && distinct_classes.len() == 2 {
            // Ties are broken by lowest class id, which biases a binary vote with even k.
            context.log_message(
                &format!(
                    "KNN k = {k} is even for a 2-class problem; tied votes resolve to the lowest class id. Use an odd k or enable Distance Weighted."
                ),
                LogLevel::Warn,
            );
        }

        context.log_message(
            &format!(
                "KNN model embeds {n_rows} training rows x {n_features} features (~{} KiB) and carries them into every saved model file",
                (features.len() * std::mem::size_of::<f64>()) / 1024
            ),
            LogLevel::Info,
        );

        let model = MLModel::KnnClassifier(ModelWithMeta {
            model: KnnModel {
                features,
                n_features,
                targets,
                k,
                distance_weighted,
            },
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
                    "Column Containing the Class Labels to Fit the Classifier on",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
