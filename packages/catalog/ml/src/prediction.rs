//! Node for Making Predictions with MLModels
//!
//! This node loads a dataset (currently from a Database), transforms it into a prediction dataset,
//! and uses the trained model to make predictions.
//!
//! Supports batch processing for large datasets.
//! Adds / upserts predictions back into the Database.

use crate::ml::MLPrediction;
use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::make_new_field;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::pin::ValueType;
use flow_like::flow::{board::Board, node::remove_pin_by_name};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::arrow_schema::Schema;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_storage::lancedb::table::NewColumnTransform;
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct MLPredictNode {}

impl MLPredictNode {
    pub fn new() -> Self {
        MLPredictNode {}
    }
}

#[async_trait]
impl NodeLogic for MLPredictNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_predict",
            "Predict",
            "Predict with Machine Learning Model",
            "AI/ML",
        );
        node.set_flowscript_name("ml", "predict");
        node.set_receiver("model");
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that starts prediction",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Trained ML model to use for inference",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose the input type for prediction (database rows or raw vector)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string(), "Vector".to_string()]) // , "CSV".to_string()
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Number of records to process per batch (default: 5000, 0 = process all at once)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once predictions are written or returned",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let node_model: NodeMLModel = context.evaluate_pin("model").await?;

        // load dataset
        match source.as_str() {
            "Database" => {
                // fetch additional inputs
                let node_database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let predictions_col: String = context.evaluate_pin("predictions_col").await?;
                let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(5000);
                let batch_size = if batch_size <= 0 {
                    usize::MAX
                } else {
                    batch_size as usize
                };

                // fetch database
                let cached_db = node_database.load(context).await?;
                let database = cached_db.db.clone();

                // get schema and validate columns exist
                let existing_cols: HashSet<String> = {
                    let database = database.read().await;
                    let schema = database.schema().await?;
                    schema.fields.iter().map(|f| f.name().clone()).collect()
                };
                if !existing_cols.contains(&records_col) {
                    return Err(anyhow!(format!(
                        "Database doesn't contain input column `{}`!",
                        records_col
                    )));
                }

                // load model once for all batches
                let model = node_model.get_model(context).await?;

                // add prediction column if missing (before batch loop)
                let mut column_added = existing_cols.contains(&predictions_col);

                let mut offset: usize = 0;
                let mut total_processed: usize = 0;
                loop {
                    // fetch batch
                    let t0 = std::time::Instant::now();
                    cached_db.ensure_flushed().await?;
                    let mut records = {
                        let database = database.read().await;
                        database
                            // Full rows: the upsert below merges with `when_matched_update_all`,
                            // which replaces the matched row wholesale, so a partial row would
                            // null out every column that was not fetched.
                            .filter("true", None, batch_size, offset)
                            .await?
                    };
                    let batch_count = records.len();
                    if batch_count == 0 {
                        break; // no more records
                    }
                    context.log_message(
                        &format!(
                            "Batch {}: fetched {} records (offset {})",
                            offset / batch_size.min(batch_count),
                            batch_count,
                            offset
                        ),
                        LogLevel::Debug,
                    );
                    context.log_message(
                        &format!("Fetch records (db): {:?}", t0.elapsed()),
                        LogLevel::Debug,
                    );

                    // predict on batch
                    let t0 = std::time::Instant::now();
                    {
                        let model_guard = model.lock().await;
                        model_guard.predict_on_values(
                            &mut records,
                            &records_col,
                            &predictions_col,
                        )?;
                    }
                    context.log_message(
                        &format!("Predict batch: {:?}", t0.elapsed()),
                        LogLevel::Debug,
                    );

                    // upsert batch
                    let t0 = std::time::Instant::now();
                    {
                        let mut database = database.write().await;
                        if !column_added {
                            let probe =
                                records.first().ok_or_else(|| anyhow!("Got No Records!"))?;
                            let new_field = make_new_field(probe, &predictions_col)?;
                            let schema = Schema::new(vec![new_field]);
                            database
                                .inner_mut()
                                .add_columns(NewColumnTransform::AllNulls(schema.into()), None)
                                .await?;
                            context.log_message(
                                &format!("Added {} as new column", predictions_col),
                                LogLevel::Debug,
                            );
                            column_added = true;
                        }
                    }
                    cached_db
                        .upsert_from(context, records, records_col.clone())
                        .await?;
                    context.log_message(
                        &format!("Upsert batch: {:?}", t0.elapsed()),
                        LogLevel::Debug,
                    );

                    total_processed += batch_count;
                    offset += batch_count;

                    // if we got less than batch_size, we're done
                    if batch_count < batch_size {
                        break;
                    }
                }

                context.log_message(
                    &format!("Processed {} total records", total_processed),
                    LogLevel::Info,
                );

                // set output
                let database_value: Value = flow_like_types::json::to_value(&node_database)?;
                context
                    .set_pin_value("database_out", database_value)
                    .await?;
            }
            "Vector" => {
                // load vector as dataset
                let vector: Vec<f64> = context.evaluate_pin("vector").await?;

                let t0 = std::time::Instant::now();
                let prediction = {
                    let model = node_model.get_model(context).await?;
                    let model_guard = model.lock().await;
                    model_guard.predict_on_vector(vector)?
                }; // drop model
                let elapsed = t0.elapsed();
                context.log_message(&format!("Predict: {elapsed:?}"), LogLevel::Debug);

                // set outputs
                context
                    .set_pin_value("prediction", json!(prediction))
                    .await?;
            }
            _ => return Err(anyhow!("Datasource Not Implemented")),
        };

        // set outputs
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
                    "Input Col",
                    "Column containing records to predict on",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("predictions_col").is_none() {
                node.add_input_pin(
                    "predictions_col",
                    "Output Col",
                    "Column that should be added for predictions",
                    VariableType::String,
                );
            }
            if node.get_pin_by_name("database_out").is_none() {
                node.add_output_pin(
                    "database_out",
                    "Database",
                    "Database Connection (Updated)",
                    VariableType::Struct,
                )
                .set_schema::<NodeDBConnection>()
                .set_options(PinOptions::new().set_enforce_schema(true).build());
            }
            remove_pin_by_name(node, "vector");
            remove_pin_by_name(node, "prediction");
        } else if source_pin == *"Vector" {
            if node.get_pin_by_name("vector").is_none() {
                node.add_input_pin("vector", "Vector", "Vector (1d Array)", VariableType::Float)
                    .set_value_type(ValueType::Array);
            }
            if node.get_pin_by_name("prediction").is_none() {
                node.add_output_pin(
                    "prediction",
                    "Prediction",
                    "Model Prediction as Struct",
                    VariableType::Struct,
                )
                .set_schema::<MLPrediction>();
            }
            remove_pin_by_name(node, "database");
            remove_pin_by_name(node, "records");
            remove_pin_by_name(node, "predictions_col");
            remove_pin_by_name(node, "database_out");
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
            return;
        }
    }
}
