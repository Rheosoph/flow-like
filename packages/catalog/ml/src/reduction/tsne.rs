//! Node for **t-SNE Dimensionality Reduction**
//!
//! t-SNE is transductive: there is no reusable model, the embedding only exists for the rows it
//! was computed on. The node therefore writes the embedding back into the source table instead of
//! persisting an MLModel.

#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, values_to_array2_f64};
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
use linfa::traits::Transformer;
#[cfg(feature = "execute")]
use linfa_tsne::TSneParams;
#[cfg(feature = "execute")]
use std::collections::HashSet;
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Barnes-Hut partitions the embedding space into a tree with 2^d children per cell, so beyond 3
/// dimensions only the exact gradient (`approx_threshold = 0`) stays tractable.
#[cfg(feature = "execute")]
const MAX_APPROX_EMBEDDING_SIZE: usize = 3;

/// Above this many rows a t-SNE run takes minutes and cannot be cancelled.
#[cfg(feature = "execute")]
const SLOW_ROW_THRESHOLD: usize = 5000;

#[crate::register_node]
#[derive(Default)]
pub struct TsneNode {}

impl TsneNode {
    pub fn new() -> Self {
        TsneNode {}
    }
}

#[async_trait]
impl NodeLogic for TsneNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_tsne",
            "t-SNE Reduction",
            "t-Distributed Stochastic Neighbor Embedding. Projects high-dimensional vectors into 2-3 dimensions for visualization and writes the embedding back into the source table. t-SNE is transductive, so it produces no reusable model.",
            "AI/ML/Reduction",
        );
        node.set_flowscript_name("ml", "fitTsne");
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_version(2);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(2)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins the t-SNE embedding",
            VariableType::Execution,
        );

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose which backend supplies the data",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "embedding_size",
            "Embedding Size",
            "Dimensionality of the embedding. Must not exceed the width of the input vectors; values above 3 require the exact gradient (Approx Threshold = 0).",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 3.)).build())
        .set_default_value(Some(json!(2)));

        node.add_input_pin(
            "perplexity",
            "Perplexity",
            "Effective number of neighbors per point (typically 5-50). t-SNE requires 3 * perplexity <= rows - 1, so small tables need a small perplexity.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((1., 200.)).build())
        .set_default_value(Some(json!(30.0)));

        node.add_input_pin(
            "approx_threshold",
            "Approx Threshold",
            "Barnes-Hut theta. 0 runs the exact O(n^2) gradient, larger values approximate distant points by their cell centroid and run faster.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 2.)).build())
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "max_iter",
            "Max Iterations",
            "Number of gradient descent iterations. Fewer iterations finish sooner but may leave the embedding unconverged.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((50., 10000.)).build())
        .set_default_value(Some(json!(1000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the t-SNE embedding has been written back",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let embedding_size: i64 = context.evaluate_pin("embedding_size").await?;
        let perplexity: f64 = context.evaluate_pin("perplexity").await?;
        let approx_threshold: f64 = context.evaluate_pin("approx_threshold").await?;
        let max_iter: i64 = context.evaluate_pin("max_iter").await?;

        if embedding_size < 1 {
            return Err(anyhow!(
                "Embedding Size must be at least 1, got {embedding_size}"
            ));
        }
        if max_iter < 1 {
            return Err(anyhow!("Max Iterations must be at least 1, got {max_iter}"));
        }
        if !perplexity.is_finite() || perplexity <= 0.0 {
            return Err(anyhow!(
                "Perplexity must be a positive number, got {perplexity}"
            ));
        }
        if !approx_threshold.is_finite() || approx_threshold < 0.0 {
            return Err(anyhow!(
                "Approx Threshold must be zero or positive, got {approx_threshold}"
            ));
        }

        let embedding_size = embedding_size as usize;
        let max_iter = max_iter as usize;

        if approx_threshold > 0.0 && embedding_size > MAX_APPROX_EMBEDDING_SIZE {
            return Err(anyhow!(
                "Embedding Size {embedding_size} is not supported by the Barnes-Hut approximation: the space-partitioning tree allocates 2^{embedding_size} children per cell. Reduce the Embedding Size to {MAX_APPROX_EMBEDDING_SIZE} or set Approx Threshold to 0 for the exact gradient."
            ));
        }

        match source.as_str() {
            "Database" => {
                let node_database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let output_col: String = context.evaluate_pin("output_col").await?;

                let cached_db = node_database.load(context).await?;
                let database = cached_db.db.clone();

                let t0 = std::time::Instant::now();
                cached_db.ensure_flushed().await?;
                let records = {
                    let database = database.read().await;
                    let schema = database.schema().await?;
                    let existing_cols: HashSet<String> =
                        schema.fields.iter().map(|f| f.name().clone()).collect();
                    if !existing_cols.contains(&records_col) {
                        return Err(anyhow!(
                            "Database doesn't contain input column `{}`!",
                            records_col
                        ));
                    }
                    database
                        // Full rows: the upsert below merges with `when_matched_update_all`,
                        // which replaces the matched row wholesale, so a partial row would null
                        // out every column that was not fetched.
                        .filter("true", None, MAX_ML_PREDICTION_RECORDS, 0)
                        .await?
                };
                context.log_message(
                    &format!("Loaded {} records from database", records.len()),
                    LogLevel::Debug,
                );
                context.log_message(
                    &format!("Fetch records: {:?}", t0.elapsed()),
                    LogLevel::Debug,
                );

                let t0 = std::time::Instant::now();
                // `values_to_array2_f64` allocates one contiguous `Vec` and shapes it, which is
                // required: linfa-tsne calls `data.as_slice_mut().unwrap()` and panics on any
                // non-standard-layout array, so this must never become a view or a transpose.
                let array = values_to_array2_f64(&records, &records_col)?;
                let (rows, cols) = array.dim();
                context.log_message(
                    &format!("Preprocess data: {:?}", t0.elapsed()),
                    LogLevel::Debug,
                );

                if rows == 0 {
                    return Err(anyhow!("No records to embed"));
                }
                if embedding_size > cols {
                    return Err(anyhow!(
                        "Embedding Size {embedding_size} exceeds the input dimensionality: column `{records_col}` holds {cols}-dimensional vectors"
                    ));
                }
                let max_perplexity = (rows - 1) as f64 / 3.0;
                if perplexity > max_perplexity {
                    return Err(anyhow!(
                        "Perplexity {perplexity} is too large for {rows} rows: t-SNE requires 3 * perplexity <= rows - 1, so {rows} rows allow at most a perplexity of {max_perplexity:.2}"
                    ));
                }
                if rows > SLOW_ROW_THRESHOLD {
                    context.log_message(
                        &format!(
                            "t-SNE on {rows} rows will take a long time and cannot be cancelled once started; consider sampling the table down to a few thousand rows"
                        ),
                        LogLevel::Warn,
                    );
                }

                context.log_message(
                    &format!(
                        "Starting t-SNE on {rows}x{cols} matrix (embedding_size={embedding_size}, perplexity={perplexity}, approx_threshold={approx_threshold}, max_iter={max_iter}). The solver reports no progress until it finishes."
                    ),
                    LogLevel::Info,
                );

                let t0 = std::time::Instant::now();
                // bhtsne::run is a synchronous, uncancellable CPU loop, so it must not run on an
                // async worker thread.
                let transformed = tokio::task::spawn_blocking(move || {
                    TSneParams::embedding_size(embedding_size)
                        .perplexity(perplexity)
                        .approx_threshold(approx_threshold)
                        .max_iter(max_iter)
                        .transform(array)
                })
                .await
                .map_err(|err| anyhow!("t-SNE task failed: {err}"))?
                .map_err(|err| anyhow!("t-SNE failed: {err}"))?;
                context.log_message(&format!("Fit t-SNE: {:?}", t0.elapsed()), LogLevel::Debug);

                let t0 = std::time::Instant::now();
                let mut updated_records = records;
                for (i, row) in transformed.outer_iter().enumerate() {
                    let embedded_vec: Vec<f64> = row.iter().copied().collect();
                    if let Some(Value::Object(map)) = updated_records.get_mut(i) {
                        map.insert(output_col.clone(), json!(embedded_vec));
                    }
                }
                context.log_message(
                    &format!("Build output records: {:?}", t0.elapsed()),
                    LogLevel::Debug,
                );

                let t0 = std::time::Instant::now();
                {
                    let mut database = database.write().await;
                    let schema = database.schema().await?;
                    let existing_cols: HashSet<String> =
                        schema.fields.iter().map(|f| f.name().clone()).collect();

                    if !existing_cols.contains(&output_col) {
                        // `make_new_field` can only type scalar columns; the embedding is always a
                        // list of f64, so the arrow field is built explicitly here.
                        let item = Field::new("item", DataType::Float64, true);
                        let new_field =
                            Field::new(output_col.as_str(), DataType::List(Arc::new(item)), true);
                        let schema = Schema::new(vec![new_field]);
                        database
                            .inner_mut()
                            .add_columns(NewColumnTransform::AllNulls(schema.into()), None)
                            .await?;
                        context.log_message(
                            &format!("Added {} as new column", output_col),
                            LogLevel::Debug,
                        );
                    }
                }
                cached_db
                    .upsert_from(context, updated_records, records_col.clone())
                    .await?;
                context.log_message(
                    &format!("Upsert records: {:?}", t0.elapsed()),
                    LogLevel::Debug,
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
                    "Column containing the feature vectors",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("output_col").is_none() {
                node.add_input_pin(
                    "output_col",
                    "Output Col",
                    "Column name for the t-SNE embedding",
                    VariableType::String,
                )
                .set_default_value(Some(json!("tsne_vector")));
            }
            if node.get_pin_by_name("database_out").is_none() {
                node.add_output_pin(
                    "database_out",
                    "Database",
                    "Database with the added embedding column",
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
