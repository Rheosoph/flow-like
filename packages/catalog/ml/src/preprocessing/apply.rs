//! Node for Applying a **Fitted Transformer**
//!
//! The counterpart to the Predict node for models that reshape features instead of predicting a
//! target (feature scalers, TF-IDF vectorizers). Reads a source column in batches, writes one
//! vector per row into a target column and upserts the result back into the database, so a scaler
//! fitted on the training table can be replayed unchanged on held-out data.

#[cfg(feature = "execute")]
use crate::ml::MLModel;
use crate::ml::NodeMLModel;
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
use flow_like_storage::arrow_schema::{DataType, Field, Schema};
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
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Width of the vector a transformed row carries in `column`.
#[cfg(feature = "execute")]
fn vector_dimension(probe: &Value, column: &str) -> Result<usize> {
    probe
        .get(column)
        .and_then(|value| value.as_array())
        .map(|vector| vector.len())
        .ok_or_else(|| anyhow!("Expected column `{column}` to hold one vector per row"))
}

/// Arrow field for a column holding one Float64 vector per row.
///
/// `crate::ml::make_new_field` maps scalars only and errors out on a `Value::Array`, so the
/// transformer output needs its own field. The fixed-size list matches how LanceDB stores vector
/// columns elsewhere in the codebase and keeps the column indexable.
#[cfg(feature = "execute")]
fn vector_field(probe: &Value, column: &str) -> Result<Field> {
    let dimension = vector_dimension(probe, column)?;
    let dimension = i32::try_from(dimension)
        .ok()
        .filter(|dimension| *dimension > 0)
        .ok_or_else(|| {
            anyhow!("Transform produced an unusable vector width for column `{column}`")
        })?;
    Ok(Field::new(
        column,
        DataType::FixedSizeList(
            Arc::new(Field::new("item", DataType::Float64, true)),
            dimension,
        ),
        true,
    ))
}

/// Rejects an existing target column that cannot hold a vector per row, so the user gets a column
/// level error instead of a serialization failure deep inside the writer.
#[cfg(feature = "execute")]
fn ensure_vector_column(existing: &DataType, dimension: usize, column: &str) -> Result<()> {
    match existing {
        DataType::FixedSizeList(_, width) if *width as usize != dimension => Err(anyhow!(
            "Column `{column}` stores vectors of width {width}, but this transform produces {dimension}"
        )),
        DataType::FixedSizeList(_, _) | DataType::List(_) | DataType::LargeList(_) => Ok(()),
        other => Err(anyhow!(
            "Column `{column}` already exists as {other:?}, but Apply Transform writes one vector per row. Choose a free output column."
        )),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MLApplyTransformNode {}

impl MLApplyTransformNode {
    pub fn new() -> Self {
        MLApplyTransformNode {}
    }
}

#[async_trait]
impl NodeLogic for MLApplyTransformNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_apply_transform",
            "Apply Transform",
            "Apply a fitted transformer (Feature Scaler, TF-IDF) to a table, writing one vector per row. A Feature Scaler replays the exact offsets and scales learned at fit time, so applying it to train and test gives both the same statistics. TF-IDF is different: linfa recomputes the inverse document frequencies from the table being transformed, so vectors are only comparable within a single Apply Transform run.",
            "AI/ML/Preprocessing",
        );
        node.set_flowscript_name("ml", "applyTransform");
        node.set_receiver("model");
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
            "Execution trigger that starts the transform",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Fitted transformer to apply. Classifiers and regressors belong on the Predict node.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose which backend supplies the rows to transform",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Number of records to transform per batch (default: 5000, 0 = process all at once)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once every batch is transformed and written",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let node_model: NodeMLModel = context.evaluate_pin("model").await?;

        match source.as_str() {
            "Database" => {
                let node_database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let output_col: String = context.evaluate_pin("output_col").await?;
                let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(5000);
                let batch_size = if batch_size <= 0 {
                    usize::MAX
                } else {
                    batch_size as usize
                };

                let model = node_model.get_model(context).await?;

                // linfa's `FittedTfIdfVectorizer::transform` recomputes the IDF weights from
                // whatever rows it is handed rather than replaying the ones learned at fit time
                // (linfa-preprocessing-0.8.1/src/tf_idf_vectorization.rs:217-249). Splitting the
                // table into batches would therefore scale the same document differently
                // depending on which batch it landed in, so TF-IDF is always transformed whole.
                let batch_size = {
                    let guard = model.lock().await;
                    if matches!(&*guard, MLModel::TfIdfVectorizer(_)) {
                        if batch_size != usize::MAX {
                            context.log_message(
                                "TF-IDF weights depend on the corpus being transformed, so Batch Size is ignored and the table is processed in one pass",
                                LogLevel::Warn,
                            );
                        }
                        usize::MAX
                    } else {
                        batch_size
                    }
                };

                // Checked before any database work: an empty table would otherwise let a wrongly
                // wired classifier finish silently.
                let expected_features = {
                    let model_guard = model.lock().await;
                    if !model_guard.is_transformer() {
                        return Err(anyhow!(
                            "{} is not a transformer. Use the Predict node instead of Apply Transform.",
                            model_guard.label()
                        ));
                    }
                    match &*model_guard {
                        // linfa scales through a Zip over the fitted columns and panics on any
                        // other width, so every batch is measured before it is transformed.
                        MLModel::FeatureScaler(scaler) => Some(scaler.model.offsets().len()),
                        _ => None,
                    }
                };

                let cached_db = node_database.load(context).await?;
                let database = cached_db.db.clone();

                let (existing_cols, existing_output_type) = {
                    let database = database.read().await;
                    let schema = database.schema().await?;
                    let names: HashSet<String> =
                        schema.fields.iter().map(|f| f.name().clone()).collect();
                    let output_type = schema
                        .fields
                        .iter()
                        .find(|field| field.name().as_str() == output_col.as_str())
                        .map(|field| field.data_type().clone());
                    (names, output_type)
                };
                if !existing_cols.contains(&records_col) {
                    return Err(anyhow!(
                        "Database doesn't contain input column `{}`!",
                        records_col
                    ));
                }
                // The write below is a merge keyed on the input column. Writing the result back
                // over that same column would change the key, so every row would be inserted
                // rather than updated: the table would grow by one batch per pass and the paging
                // loop would never reach its final short batch.
                if output_col == records_col {
                    return Err(anyhow!(
                        "Output Col `{output_col}` must differ from Input Col. Apply Transform merges on the input column, so writing over it would duplicate every row instead of updating it."
                    ));
                }

                let mut column_added = existing_cols.contains(&output_col);
                let mut output_column_checked = false;
                let mut offset: usize = 0;
                let mut total_processed: usize = 0;
                loop {
                    let t0 = std::time::Instant::now();
                    cached_db.ensure_flushed().await?;
                    // Full rows, not just the input column. The upsert below merges with
                    // `when_matched_update_all`, which REPLACES the matched row wholesale — so a
                    // partial row either fails to write (non-nullable column missing) or silently
                    // nulls every column that was not fetched.
                    let mut records = {
                        let database = database.read().await;
                        database.filter("true", None, batch_size, offset).await?
                    };
                    let batch_count = records.len();
                    if batch_count == 0 {
                        break;
                    }
                    context.log_message(
                        &format!(
                            "Fetched {} records at offset {}: {:?}",
                            batch_count,
                            offset,
                            t0.elapsed()
                        ),
                        LogLevel::Debug,
                    );

                    if let Some(expected) = expected_features {
                        let width = vector_dimension(
                            records.first().ok_or_else(|| anyhow!("Got No Records!"))?,
                            &records_col,
                        )?;
                        if width != expected {
                            return Err(anyhow!(
                                "The scaler was fitted on {expected} features but column `{records_col}` holds {width} per row. Transform the same feature layout the scaler was fitted on."
                            ));
                        }
                    }

                    let t0 = std::time::Instant::now();
                    {
                        let model_guard = model.lock().await;
                        model_guard.transform_on_values(&mut records, &records_col, &output_col)?;
                    }
                    context.log_message(
                        &format!("Transform batch: {:?}", t0.elapsed()),
                        LogLevel::Debug,
                    );

                    let t0 = std::time::Instant::now();
                    {
                        let probe = records.first().ok_or_else(|| anyhow!("Got No Records!"))?;
                        let dimension = vector_dimension(probe, &output_col)?;
                        if !output_column_checked {
                            if let Some(existing) = &existing_output_type {
                                ensure_vector_column(existing, dimension, &output_col)?;
                            }
                            output_column_checked = true;
                        }
                        if !column_added {
                            let new_field = vector_field(probe, &output_col)?;
                            let schema = Schema::new(vec![new_field]);
                            let mut database = database.write().await;
                            database
                                .inner_mut()
                                .add_columns(NewColumnTransform::AllNulls(schema.into()), None)
                                .await?;
                            context.log_message(
                                &format!(
                                    "Added {} as a Float64 vector column of width {}",
                                    output_col, dimension
                                ),
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

                    if batch_count < batch_size {
                        break;
                    }
                }

                context.log_message(
                    &format!(
                        "Transformed {} records into `{}`",
                        total_processed, output_col
                    ),
                    LogLevel::Info,
                );

                let database_value: Value = flow_like_types::json::to_value(&node_database)?;
                context
                    .set_pin_value("database_out", database_value)
                    .await?;
            }
            _ => return Err(anyhow!("Datasource Not Implemented")),
        }

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
                    "Column holding the values to transform: feature vectors for a scaler, text for a TF-IDF vectorizer",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("output_col").is_none() {
                node.add_input_pin(
                    "output_col",
                    "Output Col",
                    "Column that receives the transformed vector, created as a Float64 vector column when missing",
                    VariableType::String,
                )
                .set_default_value(Some(json!("scaled_vector")));
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
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
