use crate::data::query_params as params;
use crate::data::datafusion::query::batches_to_csv_table;
use crate::data::datafusion::session::DataFusionSession;
use crate::data::excel::CSVTable;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

/// Table info returned by list tables
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TableInfo {
    /// Table name (use this for queries)
    pub name: String,
    /// Full qualified name (catalog.schema.table)
    pub qualified_name: String,
    /// Catalog name
    pub catalog: String,
    /// Schema name
    pub schema: String,
}

/// List all tables in a DataFusion session - designed for agent use
#[crate::register_node]
#[derive(Default)]
pub struct ListTablesNode {}

impl ListTablesNode {
    pub fn new() -> Self {
        ListTablesNode {}
    }
}

#[async_trait]
impl NodeLogic for ListTablesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_list_tables",
            "List Tables",
            "List all tables registered in a DataFusion session. Returns array of table names.",
            "Data/DataFusion/Tools",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to query",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution completed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "tables",
            "Tables",
            "Array of table info objects",
            VariableType::Struct,
        )
        .set_schema::<TableInfo>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "table_names",
            "Table Names",
            "Simple array of table names (for queries)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "summary",
            "Summary",
            "Human-readable summary of available tables",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 10,
            governance: 9,
            reliability: 10,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let cached_session = session.load(context).await?;

        let catalog_names = cached_session.ctx.catalog_names();
        let mut tables: Vec<TableInfo> = Vec::new();
        let mut table_names: Vec<String> = Vec::new();

        for catalog_name in catalog_names {
            if let Some(catalog) = cached_session.ctx.catalog(&catalog_name) {
                for schema_name in catalog.schema_names() {
                    if let Some(schema) = catalog.schema(&schema_name) {
                        for table_name in schema.table_names() {
                            tables.push(TableInfo {
                                name: table_name.clone(),
                                qualified_name: format!(
                                    "{}.{}.{}",
                                    catalog_name, schema_name, table_name
                                ),
                                catalog: catalog_name.clone(),
                                schema: schema_name.clone(),
                            });
                            table_names.push(table_name);
                        }
                    }
                }
            }
        }

        let summary = if tables.is_empty() {
            "No tables registered in the session.".to_string()
        } else {
            let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
            format!("Available tables: {}", names.join(", "))
        };

        context.set_pin_value("tables", json!(tables)).await?;
        context
            .set_pin_value("table_names", json!(table_names))
            .await?;
        context.set_pin_value("summary", json!(summary)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

/// Describe a table schema in a DataFusion session - designed for agent use
#[crate::register_node]
#[derive(Default)]
pub struct DescribeTableNode {}

impl DescribeTableNode {
    pub fn new() -> Self {
        DescribeTableNode {}
    }
}

#[async_trait]
impl NodeLogic for DescribeTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_describe_table",
            "Describe Table",
            "Get the schema (column names and types) of a table in a DataFusion session.",
            "Data/DataFusion/Tools",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session containing the table",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name of the table to describe",
            VariableType::String,
        )
        .set_default_value(Some(json!("data")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution completed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "schema",
            "Schema",
            "Table schema description (column names and types)",
            VariableType::String,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 10,
            governance: 9,
            reliability: 9,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;

        let cached_session = session.load(context).await?;

        let df = cached_session
            .ctx
            .sql(&format!("DESCRIBE {}", table_name))
            .await?;
        let batches = df.collect().await?;

        let mut result = format!("Schema for table '{}':\n", table_name);
        for batch in &batches {
            for row_idx in 0..batch.num_rows() {
                let col_name = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<flow_like_storage::datafusion::arrow::array::StringArray>()
                    .map(|arr| arr.value(row_idx))
                    .unwrap_or("?");
                let col_type = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<flow_like_storage::datafusion::arrow::array::StringArray>()
                    .map(|arr| arr.value(row_idx))
                    .unwrap_or("?");
                result.push_str(&format!("  - {} ({})\n", col_name, col_type));
            }
        }

        context.set_pin_value("schema", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

/// Execute SQL and return results as markdown - designed for agent use
#[crate::register_node]
#[derive(Default)]
pub struct ExecuteSqlNode {}

impl ExecuteSqlNode {
    pub fn new() -> Self {
        ExecuteSqlNode {}
    }
}

#[async_trait]
impl NodeLogic for ExecuteSqlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_execute_sql",
            "Execute SQL",
            "Execute a SQL query and return results as formatted text. Ideal for agent-driven data exploration.",
            "Data/DataFusion/Tools",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to query",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "query",
            "Query",
            "SQL query to execute. Use $placeholders for values that come from the flow (SELECT * FROM users WHERE id = $user_id) — each one adds an input pin to wire the value into. Placeholders stand for values only; table and column names cannot be parameterized.",
            VariableType::String,
        )
        .set_default_value(Some(json!("SELECT * FROM data LIMIT 10")));

        params::add_params_pin(&mut node, params::SqlFlavor::Query);

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution completed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Query results formatted as markdown table",
            VariableType::String,
        );

        node.add_output_pin(
            "table",
            "Table",
            "Query results as CSVTable for further processing",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>();

        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of rows returned",
            VariableType::Integer,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 8,
            performance: 7,
            governance: 8,
            reliability: 8,
            cost: 10,
        });

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "query", board, params::SqlFlavor::Query);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let query: String = context.evaluate_pin("query").await?;
        let query_params = params::resolve_params(context, &query, params::SqlFlavor::Query).await?;

        let cached_session = session.load(context).await?;

        context.log_message(&format!("Executing SQL: {}", query), LogLevel::Debug);

        let df = cached_session.ctx.sql(&query).await?;
        let df = params::bind(df, &query_params)?;
        let batches = df.collect().await?;

        let csv_table = batches_to_csv_table(&batches)?;
        let row_count = csv_table.row_count();

        let markdown_result = if row_count == 0 {
            "Query returned no results.".to_string()
        } else {
            let mut result = format!("Query returned {} rows:\n\n", row_count);
            result.push_str(&format_table_as_markdown(&csv_table));
            result
        };

        context
            .set_pin_value("result", json!(markdown_result))
            .await?;
        context.set_pin_value("table", json!(csv_table)).await?;
        context
            .set_pin_value("row_count", json!(row_count as i64))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

fn format_table_as_markdown(table: &CSVTable) -> String {
    let headers = table.headers();
    let rows = table.rows_as_strings();

    if headers.is_empty() {
        return "Empty table".to_string();
    }

    let mut result = String::new();

    result.push('|');
    for h in &headers {
        result.push_str(&format!(" {} |", h));
    }
    result.push('\n');

    result.push('|');
    for _ in &headers {
        result.push_str(" --- |");
    }
    result.push('\n');

    let max_rows = 50;
    for (idx, row) in rows.iter().enumerate() {
        if idx >= max_rows {
            result.push_str(&format!("\n... and {} more rows", rows.len() - max_rows));
            break;
        }
        result.push('|');
        for cell in row {
            result.push_str(&format!(" {} |", cell));
        }
        result.push('\n');
    }

    result
}
