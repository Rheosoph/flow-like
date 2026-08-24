use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_storage::databases::vector::{
    buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
};
use flow_like_types::{Cacheable, Value, async_trait, sync::RwLock};
use std::sync::Arc;

pub use flow_like_catalog_core::{CachedDB, NodeDBConnection};

pub mod add_column;
pub mod count;
pub mod delete;
pub mod drop_column;
pub mod drop_index;
pub mod drop_table;
pub mod filter;
pub mod flush;
pub mod fts_search;
pub mod hybrid_search;
pub mod index;
pub mod insert;
pub mod list;
pub mod list_indices;
pub mod list_tables;
pub mod make_column_optional;
pub mod open_remote;
pub mod optimize;
pub mod purge;
pub mod schema;
pub mod upsert;
pub mod vector_search;

#[crate::register_node]
#[derive(Default)]
pub struct CreateLocalDatabaseNode {}

impl CreateLocalDatabaseNode {
    pub fn new() -> Self {
        CreateLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for CreateLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "open_local_db",
            "Open Database",
            "Open a local database",
            "Data/Database",
        );
        node.set_flowscript_name("db", "open");
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "name",
            "Table Name",
            "Name of the Table",
            VariableType::String,
        );
        node.add_input_pin(
            "user_scoped",
            "User Scoped",
            "Store database in user directory instead of project directory",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(false)));

        node.add_input_pin(
            "batch_size",
            "Batch Size",
            "Number of items to buffer before flushing writes to storage. 0 = no buffering.",
            VariableType::Integer,
        )
        .set_default_value(Some(flow_like_types::json::json!(1000)));

        node.add_output_pin(
            "exec_out",
            "Created Database",
            "Done Creating Database",
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

        let table: String = context.evaluate_pin("name").await?;
        let table = table.trim().to_string();
        LanceDBVectorStore::validate_table_name(&table)?;
        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);
        let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(1000);
        let batch_size = batch_size.max(0) as usize;
        let cache_key = if user_scoped {
            format!("db_user_{}", table)
        } else {
            format!("db_{}", table)
        };
        let cache_set = context.cache.read().await.contains_key(&cache_key);
        if !cache_set {
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
                    .ok_or(flow_like_types::anyhow!("No database builder found"))?(
                    board_dir
                )
            };

            let db = context.app_state.with_lance_session(db).execute().await?;
            let mut lance_store = LanceDBVectorStore::from_connection(db, table).await;
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

            // Register a completion callback to flush remaining buffered writes.
            // The cached store retains each write's originating node so a
            // deferred failure is visible on the writer, not just stderr.
            let completion_db = cached.clone();
            let fallback_origin = CachedDB::write_origin(context);
            context
                .hook_completion_event(Arc::new(move |run| {
                    let db = completion_db.clone();
                    let fallback_origin = fallback_origin.clone();
                    Box::pin(async move { db.flush_on_completion(run, &fallback_origin).await })
                }))
                .await;

            let cacheable: Arc<dyn Cacheable> = Arc::new(cached.clone());
            context
                .cache
                .write()
                .await
                .insert(cache_key.clone(), cacheable);
        }

        let db = NodeDBConnection { cache_key };

        let db: Value = flow_like_types::json::to_value(&db)?;

        context.set_pin_value("database", db).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
