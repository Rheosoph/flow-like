//! Node for extracting KMeans cluster centroids
//!
//! Returns the cluster centers from a trained KMeans model.

use crate::ml::{KMeansCentroids, NodeMLModel};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::json::json;
use flow_like_types::{Result, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct GetKMeansCentroidsNode {}

impl GetKMeansCentroidsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetKMeansCentroidsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_get_kmeans_centroids",
            "Get Centroids",
            "Extract cluster centroids from a trained KMeans model",
            "AI/ML/Model Info",
        );
        node.set_flowscript_name("ml", "getKmeansCentroids");
        node.set_receiver("model");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10) // Only extracts math params, no raw data
                .set_security(10) // Pure computation, no external calls
                .set_performance(9)
                .set_governance(9)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Trained KMeans model",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated when centroids are extracted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Centroids",
            "Cluster centroids with metadata",
            VariableType::Struct,
        )
        .set_schema::<KMeansCentroids>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        use crate::ml::MLModel;

        context.deactivate_exec_pin("exec_out").await?;

        let node_model: NodeMLModel = context.evaluate_pin("model").await?;
        let model_arc = node_model.get_model(context).await?;
        let model = model_arc.lock().await;

        match &*model {
            MLModel::KMeans(kmeans) => {
                let centroids = kmeans.model.centroids();
                let (n_clusters, n_dims) = centroids.dim();

                let centroids_vec: Vec<Vec<f64>> =
                    (0..n_clusters).map(|i| centroids.row(i).to_vec()).collect();

                let result = crate::ml::KMeansCentroids {
                    k: n_clusters,
                    dimensions: n_dims,
                    centroids: centroids_vec,
                };

                context.log_message(
                    &format!(
                        "Extracted {} centroids with {} dimensions each",
                        n_clusters, n_dims
                    ),
                    LogLevel::Debug,
                );

                context.set_pin_value("result", json!(result)).await?;
                context.activate_exec_pin("exec_out").await?;
                Ok(())
            }
            other => Err(flow_like_types::anyhow!(
                "Expected KMeans model, got {}",
                other
            )),
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature"
        ))
    }
}
