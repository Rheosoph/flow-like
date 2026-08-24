use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct AddColumnLocalDatabaseNode {}

impl AddColumnLocalDatabaseNode {
    pub fn new() -> Self {
        AddColumnLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for AddColumnLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "add_column_local_db",
            "Add Column",
            "Adds a column using a typed SQL expression (e.g. 0, '', CAST(NULL AS STRING)). LanceDB rejects bare NULL — wrap it in CAST(... AS <type>). Supported types: int, bigint, float, double, string, binary, boolean, date, timestamp.",
            "Data/Database/Schema",
        );
        node.set_flowscript_name("db", "addColumn");
        node.set_receiver("database");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "column_name",
            "Column Name",
            "Name of the column to add",
            VariableType::String,
        )
        .set_default_value(Some(json!("new_column")));

        node.add_input_pin(
            "sql_expression",
            "SQL Expression",
            "Typed SQL expression used to populate existing rows. Examples: 0, '', CAST(NULL AS STRING). Bare NULL is rejected; LanceDB supports int, bigint, float, double, string, binary, boolean, date, timestamp.",
            VariableType::String,
        )
        .set_default_value(Some(json!("CAST(NULL AS STRING)")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done altering schema",
            VariableType::Execution,
        );
        node.add_output_pin(
            "schema",
            "Schema",
            "Updated database schema",
            VariableType::Struct,
        )
        .set_schema::<crate::data::db::table_schema::TableSchema>();

        node.set_version(2);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;
        let sql_expression: String = context.evaluate_pin("sql_expression").await?;

        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        database
            .inner()
            .add_column(&column_name, &sql_expression)
            .await?;

        let schema = database.schema().await?;
        context.set_pin_value("schema", json!(schema)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
