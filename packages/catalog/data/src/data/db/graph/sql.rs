use crate::data::query_params as params;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # SQL Query (Graph)
/// Executes a SQL query against the graph overlay tables using DataFusion.
#[crate::register_node]
#[derive(Default)]
pub struct GraphSqlQueryNode {}

impl GraphSqlQueryNode {
    pub fn new() -> Self {
        GraphSqlQueryNode {}
    }
}

#[async_trait]
impl NodeLogic for GraphSqlQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "graph_sql_query",
            "SQL Query (Graph)",
            "Executes a read-only SQL query against graph overlay tables via DataFusion. Write any value that comes from outside the flow as a $placeholder and wire it into the pin that appears — never build the SQL string around it.",
            "Data/Database/Graph/Query",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

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
            "SQL query string. Use $placeholders for values that come from the flow (SELECT * FROM person WHERE id = $person_id) — each one adds an input pin to wire the value into. Placeholders stand for values only; table and column names cannot be parameterized.",
            VariableType::String,
        );
        params::add_params_pin(&mut node, params::SqlFlavor::Query);
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
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::databases::graph::GraphStore;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let query: String = context.evaluate_pin("query").await?;
        let query_params =
            params::resolve_params(context, &query, params::SqlFlavor::Query).await?;
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(1000);
        let limit = if limit > 0 {
            Some(limit as usize)
        } else {
            None
        };

        let store = super::load_graph_store(context, &conn.cache_key).await?;

        match store
            .sql(&query, params::to_object(&query_params), limit)
            .await
        {
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

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "query", board, params::SqlFlavor::Query);
    }
}
