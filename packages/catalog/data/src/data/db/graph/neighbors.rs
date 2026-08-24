use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Graph Neighbors
/// Finds neighbors of a node by traversing edges up to a specified depth.
#[crate::register_node]
#[derive(Default)]
pub struct GraphNeighborsNode {}

impl GraphNeighborsNode {
    pub fn new() -> Self {
        GraphNeighborsNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphNeighborsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_neighbors",
            "Graph Neighbors",
            "Finds neighbor nodes by traversing edges from a seed node",
            "Data/Database/Graph/Query",
        );
        node.set_flowscript_name("db.graph", "neighbors");
        node.set_receiver("graph");
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
            "label",
            "Node Label",
            "Label of the seed node",
            VariableType::String,
        );
        node.add_input_pin(
            "node_id",
            "Node ID",
            "ID of the seed node",
            VariableType::String,
        );
        node.add_input_pin(
            "depth",
            "Depth",
            "Maximum traversal depth (1-5)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "direction",
            "Direction",
            "Traversal direction: outgoing, incoming, or both",
            VariableType::String,
        )
        .set_default_value(Some(json!("both")));
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
            "Traversal completed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Traversal failed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "result_nodes",
            "Nodes",
            "Discovered nodes",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<flow_like_storage::databases::graph::SubgraphNode>();
        node.add_output_pin(
            "result_edges",
            "Edges",
            "Discovered edges",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<flow_like_storage::databases::graph::SubgraphEdge>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::{GraphStore, TraversalDirection};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let label: String = context.evaluate_pin("label").await?;
        let node_id: String = context.evaluate_pin("node_id").await?;
        let depth: i64 = context.evaluate_pin("depth").await.unwrap_or(1);
        let depth = (depth.max(1) as usize).min(5);
        let direction_str: String = context
            .evaluate_pin("direction")
            .await
            .unwrap_or_else(|_| "both".to_string());
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(1000);
        let limit = if limit > 0 {
            Some(limit as usize)
        } else {
            None
        };

        let direction = match direction_str.to_lowercase().as_str() {
            "outgoing" | "out" => TraversalDirection::Outgoing,
            "incoming" | "in" => TraversalDirection::Incoming,
            _ => TraversalDirection::Both,
        };

        let store = super::load_graph_store(context, &conn.cache_key).await?;
        let id_value = flow_like_types::Value::String(node_id);

        match store
            .neighbors(&label, id_value, depth, direction, limit, None)
            .await
        {
            Ok(result) => {
                context
                    .set_pin_value("result_nodes", json!(result.nodes))
                    .await?;
                context
                    .set_pin_value("result_edges", json!(result.edges))
                    .await?;
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
