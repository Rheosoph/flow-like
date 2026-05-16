use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Graph Subgraph
/// Extracts a subgraph from multiple seed nodes, ready for visualization.
#[crate::register_node]
#[derive(Default)]
pub struct GraphSubgraphNode {}

impl GraphSubgraphNode {
    pub fn new() -> Self {
        GraphSubgraphNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphSubgraphNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_subgraph",
            "Graph Subgraph",
            "Extracts a subgraph around seed nodes for visualization",
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
            "seed_labels",
            "Seed Labels",
            "Labels of seed nodes (parallel array with Seed IDs)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_input_pin(
            "seed_ids",
            "Seed IDs",
            "IDs of seed nodes (parallel array with Seed Labels)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_input_pin(
            "depth",
            "Depth",
            "Maximum traversal depth (1-5)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of results",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Subgraph extracted",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Extraction failed",
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
            "Subgraph Payload",
            "Subgraph data with nodes and edges",
            VariableType::Struct,
        );
        node.add_output_pin(
            "truncated",
            "Truncated",
            "Whether the result was truncated",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let seed_labels: Vec<String> = context.evaluate_pin("seed_labels").await?;
        let seed_ids: Vec<String> = context.evaluate_pin("seed_ids").await?;
        let depth: i64 = context.evaluate_pin("depth").await.unwrap_or(1);
        let depth = (depth.max(1) as usize).min(5);
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(1000);
        let limit = if limit > 0 {
            Some(limit as usize)
        } else {
            None
        };

        if seed_labels.len() != seed_ids.len() {
            context
                .set_pin_value(
                    "error_message",
                    json!("Seed Labels and Seed IDs must have the same length"),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let seeds: Vec<(String, flow_like_types::Value)> = seed_labels
            .into_iter()
            .zip(seed_ids)
            .map(|(label, id)| (label, flow_like_types::Value::String(id)))
            .collect();

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store.subgraph(seeds, depth, limit).await {
            Ok(result) => {
                context
                    .set_pin_value("truncated", json!(result.truncated))
                    .await?;
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
