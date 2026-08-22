use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Cypher Query
/// Executes a Cypher query against the graph overlay.
#[crate::register_node]
#[derive(Default)]
pub struct CypherQueryNode {}

impl CypherQueryNode {
    pub fn new() -> Self {
        CypherQueryNode {}
    }
}

#[async_trait]
impl NodeLogic for CypherQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_cypher_query",
            "Cypher Query",
            "Executes a Cypher query against the graph overlay",
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
            "query",
            "Query",
            "Cypher query string",
            VariableType::String,
        );
        node.add_input_pin(
            "params",
            "Parameters",
            "Query parameters (JSON object)",
            VariableType::Struct,
        )
        .set_open_schema();
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
            "Query completed",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Query failed", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );
        node.add_output_pin(
            "results",
            "Results",
            "Query results as JSON array",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_open_schema();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let query: String = context.evaluate_pin("query").await?;
        let params: flow_like_types::Value = context
            .evaluate_pin("params")
            .await
            .unwrap_or(flow_like_types::Value::Null);
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(1000);
        let limit = if limit > 0 {
            Some(limit as usize)
        } else {
            None
        };

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store.cypher(&query, params, limit).await {
            Ok(results) => {
                context.set_pin_value("results", json!(results)).await?;
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
