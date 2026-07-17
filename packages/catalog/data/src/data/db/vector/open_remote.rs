use crate::remote_util::open_remote_project_database;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_storage::databases::vector::{
    VectorStore, buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
};
use flow_like_types::{Cacheable, Value, async_trait, json::json, sync::RwLock};
use std::sync::Arc;

use super::{CachedDB, NodeDBConnection};

/// Pin names are special-cased by the flow editor to render interactive
/// dropdowns: the project pin lists all apps this app is connected to, the
/// database pin lists the shared tables of the selected project.
const PIN_REMOTE_APP_ID: &str = "_flow_remote_app_id";
const PIN_REMOTE_DATABASE: &str = "_flow_remote_database";

#[crate::register_node]
#[derive(Default)]
pub struct OpenRemoteDatabaseNode {}

impl OpenRemoteDatabaseNode {
    pub fn new() -> Self {
        OpenRemoteDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for OpenRemoteDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "open_remote_db",
            "Open Remote Database",
            "Open a shared database of a connected project. The project must have granted this app access with a role that allows reading (and for writes, writing) files or databases. Storage credentials are valid for about an hour — long-running flows with many writes should flush regularly (Flush node).",
            "Data/Database",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            PIN_REMOTE_APP_ID,
            "Project",
            "Connected project to open the database from",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            PIN_REMOTE_DATABASE,
            "Database",
            "Shared database of the selected project",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "write_access",
            "Write Access",
            "Request write access to the remote database. Requires the connection role to allow writing databases (or files).",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Number of items to buffer before flushing writes to storage. 0 = no buffering.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

        node.add_output_pin(
            "exec_out",
            "Opened Database",
            "Done opening the remote database",
            VariableType::Execution,
        );
        node.add_output_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let remote_app_id: String = context.evaluate_pin(PIN_REMOTE_APP_ID).await?;
        let table: String = context.evaluate_pin(PIN_REMOTE_DATABASE).await?;
        let write_access: bool = context.evaluate_pin("write_access").await.unwrap_or(false);
        let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(1000);
        let batch_size = batch_size.max(0) as usize;

        let remote_app_id = crate::remote_util::validate_path_id(&remote_app_id, "remote project")?;
        let table = table.trim().to_string();
        if table.is_empty() {
            return Err(flow_like_types::anyhow!(
                "No database selected on the 'Database' pin"
            ));
        }
        LanceDBVectorStore::validate_table_name(&table)?;

        let access_mode = if write_access { "write" } else { "read" };
        // "::" cannot appear in table names, so this key can never collide
        // with the local Open Database keys (db_{table} / db_user_{table}).
        let cache_key = format!("db::remote::{}::{}::{}", remote_app_id, access_mode, table);
        let cache_set = context.cache.read().await.contains_key(&cache_key);

        if !cache_set {
            // Reuse a run-scoped raw connection across every table opened
            // from this connected project.
            let db =
                open_remote_project_database(context, &remote_app_id, &table, write_access).await?;
            let mut lance_store = LanceDBVectorStore::from_connection(db, table.clone()).await;
            if let Some(opts) = &context
                .app_state
                .config
                .read()
                .await
                .callbacks
                .lance_write_options
            {
                lance_store.set_write_options(opts.clone());
            }
            let buffered = BufferedVectorStore::new(lance_store, batch_size);
            let cached = CachedDB {
                db: Arc::new(RwLock::new(buffered)),
            };

            let db_ref = cached.db.clone();
            context
                .hook_completion_event(Arc::new(move |_run| {
                    let db = db_ref.clone();
                    Box::pin(async move {
                        let mut guard = db.write().await;
                        if guard.is_dirty() {
                            guard.flush().await?;
                        }
                        Ok(())
                    })
                }))
                .await;

            let cacheable: Arc<dyn Cacheable> = Arc::new(cached.clone());
            context
                .cache
                .write()
                .await
                .insert(cache_key.clone(), cacheable);

            context.log_message(
                &format!(
                    "Opened remote database '{}' of project {} ({})",
                    table, remote_app_id, access_mode
                ),
                LogLevel::Debug,
            );
        }

        let db = NodeDBConnection { cache_key };
        let db: Value = flow_like_types::json::to_value(&db)?;

        context.set_pin_value("database", db).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
