//! Node for **PCA Dimensionality Reduction**
//!
//! This node loads a dataset (currently from a database source), transforms it using
//! Principal Component Analysis (PCA) to reduce dimensionality.

#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, make_new_field, values_to_array2_f64};
#[cfg(feature = "execute")]
use flow_like::flow::{board::Board, execution::LogLevel};
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
#[cfg(feature = "execute")]
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::{Fit, Predict};
#[cfg(feature = "execute")]
use linfa_reduction::Pca;
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct FitPcaNode {}

impl FitPcaNode {
    pub fn new() -> Self {
        FitPcaNode {}
    }
}

#[async_trait]
impl NodeLogic for FitPcaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_pca",
            "PCA Reduction",
            "Principal Component Analysis for dimensionality reduction",
            "AI/ML/Reduction",
        );
        node.set_flowscript_name("ml", "fitPca");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins PCA reduction",
            VariableType::Execution,
        );

        node.add_input_pin(
            "n_components",
            "Components",
            "Number of principal components to keep",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 1000.)).build())
        .set_default_value(Some(json!(2)));

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

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once PCA transformation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "explained_variance",
            "Explained Variance",
            "Variance explained by each principal component",
            VariableType::Float,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let n_components: usize = context.evaluate_pin("n_components").await?;

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
                let array = values_to_array2_f64(&records, &records_col)?;
                let dataset = DatasetBase::from(array);
                context.log_message(
                    &format!("Preprocess data: {:?}", t0.elapsed()),
                    LogLevel::Debug,
                );

                let t0 = std::time::Instant::now();
                let pca: Pca<f64> = Pca::params(n_components).fit(&dataset)?;
                context.log_message(&format!("Fit PCA: {:?}", t0.elapsed()), LogLevel::Debug);

                let explained_variance: Vec<f64> = pca.explained_variance().to_vec();
                context.log_message(
                    &format!("Explained variance: {:?}", explained_variance),
                    LogLevel::Debug,
                );

                let t0 = std::time::Instant::now();
                let transformed = pca.predict(&dataset);
                context.log_message(
                    &format!("Transform data: {:?}", t0.elapsed()),
                    LogLevel::Debug,
                );

                let t0 = std::time::Instant::now();
                let mut updated_records = records;
                for (i, row) in transformed.outer_iter().enumerate() {
                    let reduced_vec: Vec<f64> = row.iter().copied().collect();
                    if let Some(Value::Object(map)) = updated_records.get_mut(i) {
                        map.insert(output_col.clone(), json!(reduced_vec));
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
                        let probe = updated_records
                            .first()
                            .ok_or_else(|| anyhow!("No records to process"))?;
                        let new_field = make_new_field(probe, &output_col)?;
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
                context
                    .set_pin_value("explained_variance", json!(explained_variance))
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
                    "Column name for reduced vectors",
                    VariableType::String,
                )
                .set_default_value(Some(json!("pca_vector")));
            }
            if node.get_pin_by_name("database_out").is_none() {
                node.add_output_pin(
                    "database_out",
                    "Database",
                    "Database with added reduced vectors column",
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
