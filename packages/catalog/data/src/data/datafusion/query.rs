use crate::data::datafusion::session::DataFusionSession;
use crate::data::excel::CSVTable;
use crate::data::query_params as params;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
pub use flow_like_catalog_data_support::data::datafusion::query::*;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SqlQueryNode {}

impl SqlQueryNode {
    pub fn new() -> Self {
        SqlQueryNode {}
    }
}

#[async_trait]
impl NodeLogic for SqlQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_sql_query",
            "SQL Query",
            "Execute a SQL statement against a DataFusion session. SELECT returns results as both a CSVTable (for analytics) and array of row objects (for iteration). Registered Lance tables also accept INSERT INTO, and UPDATE/DELETE with a WHERE clause that references at least one column (constant-only conditions like WHERE true are refused, as are subqueries and multi-table forms; writes return a single `count` row). Write any value that comes from outside the flow as a $placeholder and wire it into the pin that appears — never build the SQL string around it.",
            "Data/DataFusion",
        );
        node.set_flowscript_name("df", "sqlQuery");
        node.set_receiver("session");
        node.add_icon("/flow/icons/database.svg");
        node.set_version(3);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session with registered tables",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "query",
            "Query",
            "SQL query to execute (e.g., SELECT * FROM mytable WHERE column > 10). Use $placeholders for values that come from the flow (SELECT * FROM users WHERE id = $user_id) — each one adds an input pin to wire the value into. Placeholders stand for values only; table and column names cannot be parameterized.",
            VariableType::String,
        )
        .set_default_value(Some(json!("SELECT * FROM data LIMIT 100")));

        params::add_params_pin(&mut node, params::SqlFlavor::Query);

        node.add_output_pin(
            "exec_out",
            "Done",
            "Query executed successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "table",
            "Table",
            "Query results as a CSVTable (columnar format, good for analytics)",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>();

        node.add_output_pin(
            "rows",
            "Rows",
            "Query results as array of row structs with Flow-Like-compatible values",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of rows in the result",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 8,
            governance: 9,
            reliability: 8,
            cost: 10,
        });

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "query", board, params::SqlFlavor::Query);
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let query: String = context.evaluate_pin("query").await?;

        // UPDATE/DELETE with subqueries or joined tables cannot be forwarded to
        // Lance faithfully (DataFusion only hands the table plain WHERE
        // conjuncts) — refuse those shapes before planning can mangle them.
        flow_like_storage::databases::sql_guard::validate_lance_dml_sql(&query)?;

        let query_params =
            params::resolve_params(context, &query, params::SqlFlavor::Query).await?;

        let cached_session = session.load(context).await?;

        context.log_message(&format!("Executing SQL: {}", query), LogLevel::Debug);

        let df = cached_session.ctx.sql(&query).await?;
        let df = params::bind(df, &query_params)?;
        let batches = df.collect().await?;

        let csv_table = batches_to_csv_table(&batches)?;
        let rows = batches_to_rows(&batches)?;
        let row_count = csv_table.row_count() as i64;

        context.set_pin_value("table", json!(csv_table)).await?;
        context.set_pin_value("rows", json!(rows)).await?;
        context.set_pin_value("row_count", json!(row_count)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_types::tokio;

    #[tokio::test]
    async fn test_sql_query_node_structure() {
        let node_logic = SqlQueryNode::new();
        let node = node_logic.get_node();

        assert_eq!(node.name, "df_sql_query");
        assert_eq!(node.friendly_name, "SQL Query");
        assert_eq!(node.version, Some(3));

        let input_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Input)
            .collect();
        let output_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == flow_like::flow::pin::PinType::Output)
            .collect();
        let rows_pin = output_pins.iter().find(|p| p.name == "rows").unwrap();

        assert!(input_pins.iter().any(|p| p.name == "exec_in"));
        assert!(input_pins.iter().any(|p| p.name == "session"));
        assert!(input_pins.iter().any(|p| p.name == "query"));
        assert!(input_pins.iter().any(|p| p.name == "params"));
        assert!(output_pins.iter().any(|p| p.name == "exec_out"));
        assert!(output_pins.iter().any(|p| p.name == "table"));
        assert!(output_pins.iter().any(|p| p.name == "rows"));
        assert!(output_pins.iter().any(|p| p.name == "row_count"));
        assert_eq!(rows_pin.data_type, VariableType::Struct);
        assert_eq!(rows_pin.value_type, ValueType::Array);
    }
}
