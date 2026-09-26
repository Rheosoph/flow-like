use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Lists all table names in a database location
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
            "list_tables_db",
            "List Tables",
            "Lists all available table names in the database location",
            "Data/Database/Meta",
        );
        node.set_flowscript_name("db", "listTables");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "user_scoped",
            "User Scoped",
            "List tables from user directory instead of project directory",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done listing tables",
            VariableType::Execution,
        );

        node.add_output_pin(
            "tables",
            "Tables",
            "List of table names",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);

        let list_local = context
            .app_state
            .config
            .read()
            .await
            .callbacks
            .database_table_names
            .clone();
        if let Some(list_local) = list_local {
            let tables =
                list_local(super::connection::database_path(context, user_scoped)?).await?;
            context.set_pin_value("tables", json!(tables)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let db = super::connection::open_shared(context, user_scoped).await?;
        // Listing is a connection-level metadata operation. Do not construct a
        // table-bound store with an empty sentinel name: LanceDB 0.27 validates
        // that name while opening the table and panics internally on the error.
        // `table_names` correctly returns an empty Vec when no tables exist.
        let tables = db.table_names().execute().await?;

        context.set_pin_value("tables", json!(tables)).await?;
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
