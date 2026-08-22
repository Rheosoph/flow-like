use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Graph Paths
/// Finds the shortest connections between two objects in a graph overlay.
#[crate::register_node]
#[derive(Default)]
pub struct GraphPathsNode {}

impl GraphPathsNode {
    pub fn new() -> Self {
        GraphPathsNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphPathsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_paths",
            "Find Paths",
            "Finds the shortest connections between two objects, including alternative routes",
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
            "from_label",
            "From Label",
            "Object type of the start object",
            VariableType::String,
        );
        node.add_input_pin(
            "from_id",
            "From ID",
            "Identity of the start object",
            VariableType::String,
        );
        node.add_input_pin(
            "to_label",
            "To Label",
            "Object type of the target object",
            VariableType::String,
        );
        node.add_input_pin(
            "to_id",
            "To ID",
            "Identity of the target object",
            VariableType::String,
        );
        node.add_input_pin(
            "max_depth",
            "Max Depth",
            "Maximum number of hops to search (1-5)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of objects explored during the search",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

        node.add_output_pin("exec_out", "Done", "Paths found", VariableType::Execution);
        node.add_output_pin(
            "error",
            "Error",
            "Path search failed",
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
            "Paths Payload",
            "Found paths with their nodes and edges",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin(
            "found",
            "Found",
            "Whether a connection exists within the depth limit",
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
        let from_label: String = context.evaluate_pin("from_label").await?;
        let from_id: String = context.evaluate_pin("from_id").await?;
        let to_label: String = context.evaluate_pin("to_label").await?;
        let to_id: String = context.evaluate_pin("to_id").await?;
        let max_depth: i64 = context.evaluate_pin("max_depth").await.unwrap_or(4);
        let max_depth = (max_depth.max(1) as usize).min(5);
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(1000);
        let limit = if limit > 0 {
            Some(limit as usize)
        } else {
            None
        };

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store
            .shortest_paths(
                (from_label, flow_like_types::Value::String(from_id)),
                (to_label, flow_like_types::Value::String(to_id)),
                max_depth,
                limit,
            )
            .await
        {
            Ok(result) => {
                context.set_pin_value("found", json!(result.found)).await?;
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
