use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Graph Analytics
/// Computes structural metrics over a graph overlay: object counts, connected
/// components, and the most connected and most central objects.
#[crate::register_node]
#[derive(Default)]
pub struct GraphAnalyticsNode {}

impl GraphAnalyticsNode {
    pub fn new() -> Self {
        GraphAnalyticsNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphAnalyticsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_analytics",
            "Graph Analytics",
            "Computes degree, PageRank, and connected components over a graph overlay",
            "Data/Database/Graph/Query",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "graph",
            "Graph Connection",
            "Graph connection reference",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "edge_limit",
            "Edge Limit",
            "Maximum number of edges sampled for the computation",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Analytics computed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Analytics failed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "payload",
            "Analytics Payload",
            "Metrics: counts, components, top objects by degree and PageRank",
            VariableType::Struct,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let edge_limit: i64 = context.evaluate_pin("edge_limit").await.unwrap_or(50_000);
        let edge_limit = if edge_limit > 0 {
            Some(edge_limit as usize)
        } else {
            None
        };

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store.analytics(edge_limit).await {
            Ok(result) => {
                context.set_pin_value("payload", json!(result)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}
