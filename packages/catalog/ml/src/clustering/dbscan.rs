//! Node for Fitting a **DBSCAN Density-Based Clustering Model**
//!
//! This node loads a dataset (currently from a database source), transforms it into
//! a clustering dataset, and fits DBSCAN clustering using the [`linfa`] crate.
//! Unlike KMeans, DBSCAN doesn't produce a reusable model - it assigns labels directly.

#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, values_to_array2_f64};
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
use linfa::prelude::Transformer;
#[cfg(feature = "execute")]
use linfa_clustering::Dbscan;
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct FitDbscanNode {}

impl FitDbscanNode {
    pub fn new() -> Self {
        FitDbscanNode {}
    }
}

#[async_trait]
impl NodeLogic for FitDbscanNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_dbscan",
            "Train Clustering (DBSCAN)",
            "Fit/Train DBSCAN Density-Based Clustering",
            "AI/ML/Clustering",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(5)
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
            "epsilon",
            "Epsilon",
            "Maximum distance between points in the same cluster",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.01, 10.0)).build())
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "min_points",
            "Min Points",
            "Minimum points required to form a dense region",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 100.)).build())
        .set_default_value(Some(json!(5)));

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

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once clustering completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "n_clusters",
            "Clusters",
            "Number of clusters found (excluding noise)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "n_noise",
            "Noise Points",
            "Number of points classified as noise",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let epsilon: f64 = context.evaluate_pin("epsilon").await?;
        let min_points: usize = context.evaluate_pin("min_points").await?;

        let t0 = std::time::Instant::now();
        let array = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;

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
                };
                context.log_message(
                    &format!("Loaded {} records from database", records.len()),
                    LogLevel::Debug,
                );

                values_to_array2_f64(&records, &records_col)?
            }
            _ => return Err(anyhow!("Datasource Not Implemented")),
        };
        let elapsed = t0.elapsed();
        context.log_message(&format!("Preprocess data: {elapsed:?}"), LogLevel::Debug);

        let t0 = std::time::Instant::now();
        // DBSCAN works on raw arrays, not DatasetBase
        let dataset = DatasetBase::from(array);
        let result = Dbscan::params(min_points)
            .tolerance(epsilon)
            .transform(dataset)?;
        // DBSCAN returns a DatasetBase where targets are the cluster labels
        let labels = result.targets;
        let elapsed = t0.elapsed();
        context.log_message(&format!("DBSCAN clustering: {elapsed:?}"), LogLevel::Debug);

        let mut cluster_ids: HashSet<usize> = HashSet::new();
        let mut noise_count: i64 = 0;

        for label in labels.iter() {
            match label {
                Some(cluster_id) => {
                    cluster_ids.insert(*cluster_id);
                }
                None => {
                    noise_count += 1;
                }
            }
        }

        let n_clusters = cluster_ids.len() as i64;

        context.log_message(
            &format!(
                "DBSCAN found {} clusters and {} noise points",
                n_clusters, noise_count
            ),
            LogLevel::Info,
        );

        context
            .set_pin_value("n_clusters", json!(n_clusters))
            .await?;
        context.set_pin_value("n_noise", json!(noise_count)).await?;
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
                    "Column containing the feature vectors to cluster",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
