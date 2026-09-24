use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::{
    buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, Value, sync::RwLock};
#[cfg(feature = "execute")]
use std::sync::Arc;

#[cfg(feature = "execute")]
pub use flow_like_catalog_core::CachedDB;
pub use flow_like_catalog_core::NodeDBConnection;

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
pub mod references;
pub mod schema;
pub mod update;
pub mod upsert;
pub mod vector_search;

pub(super) fn add_write_receipt_outputs(node: &mut Node) {
    node.add_output_pin(
        "write_state",
        "Write State",
        "pending means durable on this device and awaiting cloud replay; buffered means process-local batching; applied means the configured store accepted the write",
        VariableType::String,
    );
    node.add_output_pin(
        "operation_id",
        "Operation ID",
        "Durable offline operation ID, or empty when this write has no offline receipt. For chunked imports this is the last accepted chunk.",
        VariableType::String,
    );
}

#[cfg(feature = "execute")]
pub(super) async fn publish_write_receipt(
    context: &mut ExecutionContext,
    receipt: Option<flow_like_storage::databases::vector::lancedb::LocalWriteReceipt>,
    fallback: &str,
) -> flow_like_types::Result<()> {
    let (state, operation_id) = receipt.map_or_else(
        || (fallback.to_string(), String::new()),
        |receipt| (receipt.state, receipt.operation_id),
    );
    // Older pinned boards do not have these optional outputs.
    for (name, value) in [("write_state", state), ("operation_id", operation_id)] {
        if context.get_pin_by_name(name).await.is_ok() {
            context
                .set_pin_value(name, flow_like_types::json::json!(value))
                .await?;
        }
    }
    Ok(())
}

/// Cleanup operations on a table that does not exist yet have nothing to do.
/// Returns `true` (after logging a warning) when the caller should skip.
#[cfg(feature = "execute")]
pub(crate) async fn skip_missing_table(
    context: &mut ExecutionContext,
    database: &BufferedVectorStore<LanceDBVectorStore>,
    operation: &str,
) -> flow_like_types::Result<bool> {
    if database.inner().table_exists().await? {
        return Ok(false);
    }
    context.log_message(
        &format!(
            "Skipped {operation}: table '{}' does not exist yet",
            database.inner().table_name()
        ),
        flow_like::flow::execution::LogLevel::Warn,
    );
    Ok(true)
}

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
        node.set_version(3);

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

        references::add_selector_pins(&mut node);

        node.add_output_pin(
            "exec_out",
            "Created Database",
            "Done Creating Database",
            VariableType::Execution,
        );
        references::add_reference_output(&mut node);

        node.add_output_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let table: String = context.evaluate_pin("name").await?;
        let table = table.trim().to_string();
        LanceDBVectorStore::validate_table_name(&table)?;
        let user_scoped: bool = context.evaluate_pin("user_scoped").await.unwrap_or(false);
        let batch_size: i64 = context.evaluate_pin("batch_size").await.unwrap_or(1000);
        let batch_size = batch_size.max(0) as usize;
        let selector = references::read_selector(context).await?;
        let base_key = if user_scoped {
            format!("db_user_{}", table)
        } else {
            format!("db_{}", table)
        };
        let cache_key = references::selection_cache_key(&base_key, &selector)?;
        let cache_set = context.cache.read().await.contains_key(&cache_key);
        if !cache_set {
            let context_cache = context
                .execution_cache
                .clone()
                .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
            let app_id = context_cache.app_id.clone();
            let database_path = if user_scoped {
                context_cache.get_user_dir(false)?.join("db")
            } else {
                context_cache.get_storage(false)?.join("db")
            };
            let callbacks = context.app_state.config.read().await.callbacks.clone();
            let decorator = context
                .credentials
                .is_none()
                .then(|| callbacks.decorate_database.clone())
                .flatten();
            let managed = context.credentials.is_none()
                && callbacks
                    .database_table_is_managed
                    .as_ref()
                    .is_some_and(|selected| selected(&database_path, &table));
            if managed {
                LanceDBVectorStore::validate_overlay_selector(&selector)?;
                if decorator.is_none() {
                    return Err(flow_like_types::anyhow!(
                        "The selected offline table has no logical database adapter"
                    ));
                }
            }

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
                let user_dir = user_dir.join("db");
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
                let board_dir = board_dir.join("db");
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

            let db = if managed {
                // LanceDB 0.31 creates a directory namespace on connection.
                // Its optional manifest must not probe cloud storage before
                // the complete local table adapter is installed.
                db.namespace_client_property("manifest_enabled", "false")
            } else {
                db
            };
            let db = context.app_state.with_lance_session(db).execute().await?;
            let mut lance_store = if managed {
                LanceDBVectorStore::from_connection_for_overlay(db, table, selector)?
            } else if selector.branch == "main"
                && selector.version.is_none()
                && selector.tag.is_none()
                && !selector.read_only
            {
                LanceDBVectorStore::from_connection(db, table).await
            } else {
                LanceDBVectorStore::from_connection_with_selector(db, table, selector).await?
            };
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
            if let Some(decorator) = decorator {
                lance_store = decorator(database_path, lance_store).await?;
            }
            if managed && !lance_store.is_durably_managed() {
                return Err(flow_like_types::anyhow!(
                    "The selected offline table did not receive its logical database adapter"
                ));
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

        let cached = db.load(context).await?;
        let reference = references::optional_reference(cached.db.read().await.inner()).await?;
        context
            .set_pin_value("reference", flow_like_types::json::to_value(reference)?)
            .await?;

        let db: Value = flow_like_types::json::to_value(&db)?;

        context.set_pin_value("database", db).await?;
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
