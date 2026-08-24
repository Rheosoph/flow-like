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

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);

        let context_cache = context
            .execution_cache
            .clone()
            .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
        let app_id = context_cache.app_id.clone();

        let db = if let Some(credentials) = &context.credentials {
            if user_scoped {
                credentials
                    .to_db_scoped(&context_cache.sub, &app_id)
                    .await?
            } else {
                credentials.to_db(&app_id).await?
            }
        } else if user_scoped {
            let user_dir = context_cache.get_user_dir(false)?;
            let user_dir = user_dir.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_user_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No user database builder found"))?(
                user_dir
            )
        } else {
            let board_dir = context_cache.get_storage(false)?;
            let board_dir = board_dir.child("db");
            context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .build_project_database
                .clone()
                .ok_or(flow_like_types::anyhow!("No database builder found"))?(board_dir)
        };

        let db = context.app_state.with_lance_session(db).execute().await?;
        // Listing is a connection-level metadata operation. Do not construct a
        // table-bound store with an empty sentinel name: LanceDB 0.27 validates
        // that name while opening the table and panics internally on the error.
        // `table_names` correctly returns an empty Vec when no tables exist.
        let tables = db.table_names().execute().await?;

        context.set_pin_value("tables", json!(tables)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
