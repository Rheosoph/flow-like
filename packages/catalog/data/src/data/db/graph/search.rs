use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Graph Search
/// Searches graph objects by caption or identifier across the whole overlay,
/// including objects not currently loaded in a visualization.
#[crate::register_node]
#[derive(Default)]
pub struct GraphSearchNode {}

impl GraphSearchNode {
    pub fn new() -> Self {
        GraphSearchNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphSearchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_search",
            "Graph Search",
            "Searches objects by caption or identifier across the whole graph overlay",
            "Data/Database/Graph/Query",
        );
        node.set_flowscript_name("db.graph", "search");
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
            "query",
            "Query",
            "Text matched against object captions and identifiers",
            VariableType::String,
        );
        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of matches to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Search completed",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Search failed", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "result_nodes",
            "Objects",
            "Matching objects",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<flow_like_storage::databases::graph::SubgraphNode>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let query: String = context.evaluate_pin("query").await?;
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(50);
        let limit = if limit > 0 {
            Some(limit as usize)
        } else {
            None
        };

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store.search_nodes(&query, limit).await {
            Ok(result) => {
                context.set_pin_value("result_nodes", json!(result)).await?;
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
