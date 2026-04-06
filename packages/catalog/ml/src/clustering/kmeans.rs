//! Node for Fitting a **KMeans Clustering Model**
//!
//! This node loads a dataset (currently from a database source), transforms it into
//! a clustering dataset, and fits a a KMeans clustering model using the [`linfa`] crate.

#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array2_f64};
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
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_clustering::KMeans;
#[cfg(feature = "execute")]
use linfa_nn::distance::L2Dist;
#[cfg(feature = "execute")]
use std::collections::HashSet;
use std::sync::Arc;

use crate::ml::NodeMLModel;

#[crate::register_node]
#[derive(Default)]
pub struct FitKMeansNode {}

impl FitKMeansNode {
    pub fn new() -> Self {
        FitKMeansNode {}
    }
}

#[async_trait]
impl NodeLogic for FitKMeansNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_kmeans",
            "Train Clustering (KMeans)",
            "Fit/Train KMeans Clustering",
            "AI/ML/Clustering",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(7)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins clustering",
            VariableType::Execution,
        );

        node.add_input_pin(
            "cluster",
            "Cluster",
            "Choose how many centroids to fit",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 100.)).build())
        .set_default_value(Some(json!(2)));

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose which backend supplies the training data",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()]) // , "CSV".to_string()
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained KMeans model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let n_clusters: usize = context.evaluate_pin("cluster").await?;

        // load dataset
        let t0 = std::time::Instant::now();
        let ds = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;

                // fetch records
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
                    database
                        .filter(
                            "true",
                            Some(vec![records_col.to_string()]),
                            MAX_ML_PREDICTION_RECORDS,
                            0,
                        )
                        .await?
                }; // drop db
                context.log_message(
                    &format!("Loaded {} records from database", records.len()),
                    LogLevel::Debug,
                );

                let array = values_to_array2_f64(&records, &records_col)?;
                DatasetBase::from(array)
            }
            _ => return Err(anyhow!("Datasource Not Implemented")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        // train model
        let t0 = std::time::Instant::now();
        let model: KMeans<f64, L2Dist> = KMeans::params(n_clusters).fit(&ds)?;
        let elapsed = t0.elapsed();
        context.log_message(&format!("Fit model: {elapsed:?}"), LogLevel::Debug);

        // set outputs
        let model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
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
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
            return;
        }
    }
}
