//! Node for the **Silhouette Score**
//!
//! The only clustering-quality metric linfa ships. For every sample it compares the mean distance
//! to its own cluster against the mean distance to the nearest other cluster, and averages the
//! result over the dataset. Scores run from -1 (samples sit closer to a foreign cluster) through 0
//! (clusters overlap) to +1 (compact, well separated clusters), which makes it the tool for
//! choosing `k` for KMeans or comparing DBSCAN and Gaussian Mixture runs on the same data.

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
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use flow_like_types::{Value, anyhow};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::metrics::SilhouetteScore;
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct SilhouetteScoreNode {}

impl SilhouetteScoreNode {
    pub fn new() -> Self {
        SilhouetteScoreNode {}
    }
}

#[async_trait]
impl NodeLogic for SilhouetteScoreNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_silhouette_score",
            "Silhouette Score",
            "Evaluate clustering quality: how much closer each sample sits to its own cluster than to the nearest other one (-1 to +1)",
            "AI/ML/Metrics",
        );
        node.set_flowscript_name("ml", "silhouetteScore");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(4)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins the silhouette evaluation",
            VariableType::Execution,
        );

        node.add_input_pin(
            "database",
            "Database",
            "Database connection containing the feature vectors and their cluster assignments",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "features_col",
            "Feature Col",
            "Column holding the feature vectors the clustering was computed on. Distances are euclidean, so scale the features first if their ranges differ.",
            VariableType::String,
        )
        .set_default_value(Some(json!("vector")));

        node.add_input_pin(
            "labels_col",
            "Cluster Col",
            "Column holding the cluster assignment of each sample, as a string name or a non-negative integer id",
            VariableType::String,
        )
        .set_default_value(Some(json!("cluster")));

        node.add_input_pin(
            "max_samples",
            "Max Samples",
            "Upper bound on the samples used. The metric compares every sample with every other one, so the cost grows quadratically; larger sets are sub-sampled evenly.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((2., 20000.)).build())
        .set_default_value(Some(json!(2000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the silhouette evaluation completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "score",
            "Score",
            "Mean silhouette score across all evaluated samples (-1 to +1, higher is better)",
            VariableType::Float,
        );

        node.add_output_pin(
            "n_samples",
            "Samples",
            "Number of samples the score was computed on after sub-sampling",
            VariableType::Integer,
        );

        node.add_output_pin(
            "n_clusters",
            "Clusters",
            "Number of distinct clusters found in the cluster column",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use crate::ml::{MAX_ML_PREDICTION_RECORDS, values_to_array1_target, values_to_array2_f64};
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let features_col: String = context.evaluate_pin("features_col").await?;
        let labels_col: String = context.evaluate_pin("labels_col").await?;
        let max_samples: i64 = context.evaluate_pin("max_samples").await?;
        let max_samples = max_samples.clamp(2, MAX_ML_PREDICTION_RECORDS as i64) as usize;

        let records = {
            let cached_db = database.load(context).await?;
            cached_db.ensure_flushed().await?;
            let database = cached_db.db.read().await;
            let schema = database.schema().await?;
            let existing_cols: HashSet<String> =
                schema.fields.iter().map(|f| f.name().clone()).collect();

            if !existing_cols.contains(&features_col) {
                return Err(anyhow!(
                    "Database doesn't contain feature column `{}`!",
                    features_col
                ));
            }
            if !existing_cols.contains(&labels_col) {
                return Err(anyhow!(
                    "Database doesn't contain cluster column `{}`!",
                    labels_col
                ));
            }

            database
                .filter(
                    "true",
                    Some(vec![features_col.clone(), labels_col.clone()]),
                    MAX_ML_PREDICTION_RECORDS,
                    0,
                )
                .await?
        };

        let total = records.len();
        if total < 3 {
            return Err(anyhow!(
                "Silhouette score needs at least 3 samples, got {total}"
            ));
        }

        // The metric is O(n^2 * d) — every sample is compared against every other one — so cap the
        // working set. Picking indices by `i * total / max_samples` spreads the subsample evenly
        // over the fetched rows instead of biasing towards whatever the table returns first.
        let samples: Vec<Value> = if total > max_samples {
            context.log_message(
                &format!(
                    "Silhouette score scales quadratically; evaluating an even subsample of {max_samples} out of {total} records"
                ),
                LogLevel::Warn,
            );
            (0..max_samples)
                .map(|index| records[index * total / max_samples].clone())
                .collect()
        } else {
            records
        };

        let features = values_to_array2_f64(&samples, &features_col)?;
        let (labels, _class_names) = values_to_array1_target(&samples, &labels_col)?;

        let n_samples = features.nrows();
        let n_clusters = labels.iter().collect::<HashSet<_>>().len();

        if n_clusters < 2 {
            return Err(anyhow!(
                "Silhouette score needs at least 2 clusters, `{labels_col}` contains {n_clusters}. linfa returns exactly 1.0 for a single-cluster dataset, which says nothing about clustering quality."
            ));
        }
        if n_clusters >= n_samples {
            return Err(anyhow!(
                "Silhouette score needs more samples than clusters, got {n_samples} samples across {n_clusters} clusters in `{labels_col}`. Every cluster would be a singleton, and linfa scores singleton clusters as +1."
            ));
        }

        let t0 = std::time::Instant::now();
        let dataset = DatasetBase::from(features).with_targets(labels);
        let score: f64 = dataset.silhouette_score()?;

        if !score.is_finite() {
            return Err(anyhow!(
                "Silhouette score evaluated to {score}. This happens when all samples share the same coordinates, leaving both the intra- and the inter-cluster distance at zero."
            ));
        }

        context.log_message(
            &format!(
                "Silhouette Score: {score:.4} over {n_samples} samples in {n_clusters} clusters ({:?})",
                t0.elapsed()
            ),
            LogLevel::Debug,
        );

        context.set_pin_value("score", json!(score)).await?;
        context.set_pin_value("n_samples", json!(n_samples)).await?;
        context
            .set_pin_value("n_clusters", json!(n_clusters))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}
