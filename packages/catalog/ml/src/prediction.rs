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
use flow_like_storage::contracts::database::DatabaseSelector;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};
#[cfg(feature = "execute")]
use flow_like_storage::lancedb::table::NewColumnTransform;
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[cfg(feature = "execute")]
async fn snapshot_prediction_source(
    source: &LanceDBVectorStore,
    explicit_key: Option<&str>,
    predictions_col: &str,
) -> Result<LanceDBVectorStore> {
    let reference = source.reference().await?;
    let snapshot = source
        .checkout(DatabaseSelector {
            branch: reference.branch,
            version: Some(reference.version),
            tag: None,
            read_only: true,
        })
        .await?;
    if let Some(key) = explicit_key {
        if key == predictions_col {
            return Err(anyhow!("Prediction Column must differ from Key Column"));
        }
        let schema = snapshot.schema().await?;
        let field = schema
            .field_with_name(key)
            .map_err(|_| anyhow!("Database does not contain key column '{key}'"))?;
        use flow_like_storage::arrow_schema::DataType;
        if !matches!(
            field.data_type(),
            DataType::Utf8
                | DataType::LargeUtf8
                | DataType::Utf8View
                | DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64
        ) {
            return Err(anyhow!("Key Column must contain strings or integers"));
        }
        let quoted_key = format!("\"{}\"", key.replace('"', "\"\""));
        let invalid = snapshot.sql("prediction_source", &format!(
            "SELECT {quoted_key} FROM prediction_source GROUP BY {quoted_key} HAVING {quoted_key} IS NULL OR COUNT(*) > 1 LIMIT 1"
        )).await?.collect().await?;
        if invalid.iter().any(|batch| batch.num_rows() > 0) {
            return Err(anyhow!(
                "Key Column '{key}' must contain unique, non-null values"
            ));
        }
    }
    Ok(snapshot)
}

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
        node.set_version(2);
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
                let output_database: Option<NodeDBConnection> =
                    if context.get_pin_by_name("output_database").await.is_ok() {
                        context.evaluate_pin("output_database").await?
                    } else {
                        None
                    };
                let output_database = output_database.unwrap_or_else(|| node_database.clone());
                let records_col: String = context.evaluate_pin("records").await?;
                let predictions_col: String = context.evaluate_pin("predictions_col").await?;
                let id_field: String = if context.get_pin_by_name("id_field").await.is_ok() {
                    context.evaluate_pin("id_field").await?
                } else {
                    String::new()
                };
                let id_field = id_field.trim().to_string();
                if output_database.cache_key != node_database.cache_key && id_field.is_empty() {
                    return Err(anyhow!(
                        "Set Key Column when writing predictions to another database"
                    ));
                }
                let explicit_key = !id_field.is_empty();
                let id_field = if explicit_key {
                    id_field
                } else {
                    records_col.clone()
                };
                let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(5000);
                let batch_size = if batch_size <= 0 {
                    usize::MAX
                } else {
                    batch_size as usize
                };

                // fetch database
                let (cached_db, mut source_generation) =
                    node_database.load_with_generation(context).await?;
                cached_db.ensure_flushed().await?;
                let output_db = output_database.load(context).await?;
                output_db.ensure_flushed().await?;
                output_db.db.read().await.inner().ensure_writable()?;
                let mut database = {
                    let input = cached_db.db.read().await;
                    snapshot_prediction_source(
                        input.inner(),
                        explicit_key.then_some(id_field.as_str()),
                        &predictions_col,
                    )
                    .await?
                };
                if explicit_key && output_database.cache_key != node_database.cache_key {
                    let output = output_db.db.read().await;
                    if output.inner().raw().await.is_ok() {
                        snapshot_prediction_source(
                            output.inner(),
                            Some(id_field.as_str()),
                            &predictions_col,
                        )
                        .await?;
                    }
                }

                // get schema and validate columns exist
                let existing_cols: HashSet<String> = {
                    let schema = database.schema().await?;
                    schema.fields.iter().map(|f| f.name().clone()).collect()
                };
                if !existing_cols.contains(&records_col) {
                    return Err(anyhow!(format!(
                        "Database doesn't contain input column `{}`!",
                        records_col
                    )));
                }
                if !existing_cols.contains(&id_field) {
                    return Err(anyhow!("Database does not contain key column '{id_field}'"));
                }

                // load model once for all batches
                let model = node_model.get_model(context).await?;

                // add prediction column if missing (before batch loop)
                let mut column_added = {
                    let output = output_db.db.read().await;
                    match output.inner().raw().await {
                        Ok(table) => table
                            .schema()
                            .await?
                            .fields()
                            .iter()
                            .any(|field| field.name() == &predictions_col),
                        // A new destination infers its full schema on the first write.
                        Err(_) => true,
                    }
                };

                let mut offset: usize = 0;
                let mut total_processed: usize = 0;
                loop {
                    // fetch batch
                    let t0 = std::time::Instant::now();
                    let (refreshed_input, generation) =
                        node_database.load_with_generation(context).await?;
                    if generation != source_generation {
                        let connection = refreshed_input
                            .db
                            .read()
                            .await
                            .inner()
                            .connection()?
                            .clone();
                        database = database.reopen(connection).await?;
                        source_generation = generation;
                    }
                    let mut records = {
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
                    let output_db = output_database.load(context).await?;
                    {
                        let mut database = output_db.db.write().await;
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
                    output_db
                        .upsert_from(context, records, id_field.clone())
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

                output_database
                    .load(context)
                    .await?
                    .ensure_flushed()
                    .await?;

                context.log_message(
                    &format!("Processed {} total records", total_processed),
                    LogLevel::Info,
                );

                // set output
                let database_value: Value = flow_like_types::json::to_value(&output_database)?;
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
            if node.get_pin_by_name("output_database").is_none() {
                node.add_input_pin(
                    "output_database", "Output Database",
                    "Optional writable destination. Copies full source rows with predictions; leave empty to update the input database.",
                    VariableType::Struct,
                ).set_schema::<NodeDBConnection>()
                    .set_options(PinOptions::new().set_optional(true).set_enforce_schema(true).build())
                    .set_default_value(Some(json!(null)));
            }
            if node.get_pin_by_name("id_field").is_none() {
                node.add_input_pin("id_field", "Key Column", "Stable application key for prediction upserts. Required for a separate destination. Empty retains the existing input-column matching behavior.", VariableType::String)
                    .set_default_value(Some(json!("")));
            }
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
            remove_pin_by_name(node, "output_database");
            remove_pin_by_name(node, "id_field");
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
            return;
        }
    }
}

#[cfg(all(test, feature = "execute"))]
mod reference_tests {
    use super::*;
    use crate::ml::{MLModel, ModelWithMeta};
    use linfa::traits::Fit;
    use ndarray::{Array1, Array2};
    use std::path::PathBuf;

    struct TestPath(PathBuf);
    impl Drop for TestPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn database(rows: Vec<Value>) -> Result<(TestPath, LanceDBVectorStore)> {
        let path =
            std::env::temp_dir().join(format!("flow-prediction-{}", flow_like_types::create_id()));
        std::fs::create_dir_all(&path)?;
        let mut source = LanceDBVectorStore::new(path.clone(), "source".into()).await?;
        source.insert(rows).await?;
        Ok((TestPath(path), source))
    }

    #[tokio::test]
    async fn prediction_source_rejects_ambiguous_keys_before_writes() -> Result<()> {
        for rows in [
            vec![
                json!({"id": 1, "vector": [1.0]}),
                json!({"id": 1, "vector": [2.0]}),
            ],
            vec![
                json!({"id": 1, "vector": [1.0]}),
                json!({"id": null, "vector": [2.0]}),
            ],
        ] {
            let (_guard, source) = database(rows).await?;
            assert!(
                snapshot_prediction_source(&source, Some("id"), "prediction")
                    .await
                    .is_err()
            );
            assert!(
                source
                    .schema()
                    .await?
                    .field_with_name("prediction")
                    .is_err()
            );
            assert_eq!(source.count(None).await?, 2);
        }
        let (_guard, source) = database(vec![json!({"id": 1, "vector": [1.0]})]).await?;
        assert!(
            snapshot_prediction_source(&source, Some("id"), "id")
                .await
                .is_err()
        );
        assert!(
            snapshot_prediction_source(&source, Some("vector"), "prediction")
                .await
                .is_err()
        );
        assert!(
            snapshot_prediction_source(&source, Some("missing"), "prediction")
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn prediction_batches_read_one_snapshot_and_write_a_separate_branch() -> Result<()> {
        let (_guard, mut source) = database(vec![
            json!({"id": 1, "vector": [1.0], "label": "first"}),
            json!({"id": 2, "vector": [2.0], "label": "second"}),
        ])
        .await?;
        let mut output = source.create_branch("predictions").await?;
        let snapshot = snapshot_prediction_source(&source, Some("id"), "prediction").await?;
        let snapshot_version = snapshot.reference().await?.version;
        source
            .insert(vec![json!({"id": 3, "vector": [3.0], "label": "later"})])
            .await?;
        let training = linfa::Dataset::new(
            Array2::from_shape_vec((3, 1), vec![1.0, 2.0, 3.0])?,
            Array1::from(vec![2.0, 4.0, 6.0]),
        );
        let model = MLModel::LinearRegression(ModelWithMeta {
            model: linfa_linear::LinearRegression::default().fit(&training)?,
            classes: None,
        });
        let mut offset = 0;
        loop {
            let mut rows = snapshot.filter("true", None, 1, offset).await?;
            if rows.is_empty() {
                break;
            }
            model.predict_on_values(&mut rows, "vector", "prediction")?;
            if offset == 0 {
                output
                    .add_columns(
                        NewColumnTransform::AllNulls(
                            Schema::new(vec![make_new_field(&rows[0], "prediction")?]).into(),
                        ),
                        None,
                    )
                    .await?;
            }
            output.upsert(rows, "id".into()).await?;
            offset += 1;
        }
        assert_eq!(offset, 2);
        assert_eq!(source.count(None).await?, 3);
        assert_eq!(snapshot.reference().await?.version, snapshot_version);
        assert!(
            source
                .schema()
                .await?
                .field_with_name("prediction")
                .is_err()
        );
        assert_eq!(output.count(None).await?, 2);
        let first = output.filter("id = 1", None, 1, 0).await?;
        assert_eq!(first[0]["label"], "first");
        assert!((first[0]["prediction"].as_f64().expect("numeric prediction") - 2.0).abs() < 1e-8);
        Ok(())
    }
}
