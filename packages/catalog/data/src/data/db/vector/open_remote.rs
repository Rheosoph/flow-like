use crate::remote_util::open_remote_project_database_lease;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::{CachedDBRefreshHook, CachedDBRefresher};
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

#[derive(Clone)]
struct RemoteDatabaseInitializationSlot {
    lock: Arc<flow_like_types::tokio::sync::Mutex<()>>,
}

impl Cacheable for RemoteDatabaseInitializationSlot {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

struct RemoteDatabaseRefresher {
    cache_key: String,
    remote_app_id: String,
    table: String,
    write_access: bool,
    refresh_at: std::sync::Mutex<std::time::Instant>,
    refresh_lock: flow_like_types::tokio::sync::Mutex<()>,
    generation: std::sync::atomic::AtomicU64,
}

impl RemoteDatabaseRefresher {
    fn is_fresh(&self) -> bool {
        *self
            .refresh_at
            .lock()
            .expect("remote database refresh deadline lock poisoned")
            > std::time::Instant::now()
    }
}

#[async_trait]
impl CachedDBRefresher for RemoteDatabaseRefresher {
    async fn refresh(&self, context: &ExecutionContext) -> flow_like_types::Result<()> {
        if self.is_fresh() {
            return Ok(());
        }

        // One table-level refresh at a time. Different tables may arrive here
        // concurrently; the raw project-connection cache performs the second
        // level of single-flight across all of them.
        let _refresh = self.refresh_lock.lock().await;
        if self.is_fresh() {
            return Ok(());
        }

        let cached = context
            .cache
            .read()
            .await
            .get(&self.cache_key)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Remote database cache disappeared"))?;
        let cached = cached
            .as_any()
            .downcast_ref::<CachedDB>()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Remote database cache has an unexpected type")
            })?
            .clone();

        // Replace only the credential-bearing inner store. The surrounding
        // BufferedVectorStore (including any pending writes) stays intact, so
        // a run that was idle past credential expiry never tries to flush with
        // stale credentials or drops records after a failed stale flush.
        // Holding this lock also prevents operations from using the old store
        // while the replacement is being constructed.
        let mut store = cached.db.write().await;
        let lease = open_remote_project_database_lease(
            context,
            &self.remote_app_id,
            &self.table,
            self.write_access,
        )
        .await?;
        let mut lance_store =
            LanceDBVectorStore::from_connection(lease.connection, self.table.clone()).await;
        if let Some(options) = &context
            .app_state
            .config
            .read()
            .await
            .callbacks
            .lance_write_options
        {
            lance_store.set_write_options(options.clone());
        }
        *store.inner_mut() = lance_store;
        *self
            .refresh_at
            .lock()
            .expect("remote database refresh deadline lock poisoned") = lease.refresh_at;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }
}

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
            "Open a shared database of a connected project. The project must have granted this app access with a role that allows reading (and for writes, writing) files or databases. The run reuses the connection and refreshes its scoped credentials automatically.",
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
        let initialization_key = format!("{cache_key}::initialize");
        let initialization_slot = {
            let existing = context.cache.read().await.get(&initialization_key).cloned();
            if let Some(existing) = existing {
                existing
                    .as_any()
                    .downcast_ref::<RemoteDatabaseInitializationSlot>()
                    .ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "Remote database initialization cache has an unexpected type"
                        )
                    })?
                    .clone()
            } else {
                let mut cache = context.cache.write().await;
                if let Some(existing) = cache.get(&initialization_key) {
                    existing
                        .as_any()
                        .downcast_ref::<RemoteDatabaseInitializationSlot>()
                        .ok_or_else(|| {
                            flow_like_types::anyhow!(
                                "Remote database initialization cache has an unexpected type"
                            )
                        })?
                        .clone()
                } else {
                    let slot = RemoteDatabaseInitializationSlot {
                        lock: Arc::new(flow_like_types::tokio::sync::Mutex::new(())),
                    };
                    cache.insert(initialization_key, Arc::new(slot.clone()));
                    slot
                }
            }
        };

        let _initializing = initialization_slot.lock.lock().await;
        if !context.cache.read().await.contains_key(&cache_key) {
            // Reuse a run-scoped raw connection across every table opened
            // from this connected project.
            let lease =
                open_remote_project_database_lease(context, &remote_app_id, &table, write_access)
                    .await?;
            let mut lance_store =
                LanceDBVectorStore::from_connection(lease.connection, table.clone()).await;
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

            let refresher = Arc::new(RemoteDatabaseRefresher {
                cache_key: cache_key.clone(),
                remote_app_id: remote_app_id.clone(),
                table: table.clone(),
                write_access,
                refresh_at: std::sync::Mutex::new(lease.refresh_at),
                refresh_lock: flow_like_types::tokio::sync::Mutex::new(()),
                generation: std::sync::atomic::AtomicU64::new(0),
            });

            // Completion callbacks outlive this node context. Keep the pieces
            // needed to renew credentials, but detach the callback registry so
            // the callback cannot retain itself through the cloned context.
            let mut refresh_context = context.clone();
            refresh_context.completion_callbacks = Arc::new(RwLock::new(Vec::new()));
            let db_ref = cached.db.clone();
            let completion_refresher = refresher.clone();
            context
                .hook_completion_event(Arc::new(move |_run| {
                    let db = db_ref.clone();
                    let refresher = completion_refresher.clone();
                    let context = refresh_context.clone();
                    Box::pin(async move {
                        refresher.refresh(&context).await?;
                        let mut guard = db.write().await;
                        if guard.is_dirty() {
                            guard.flush().await?;
                        }
                        Ok(())
                    })
                }))
                .await;

            let cacheable: Arc<dyn Cacheable> = Arc::new(cached.clone());
            let refresh_hook: Arc<dyn Cacheable> = Arc::new(CachedDBRefreshHook::new(refresher));
            let mut cache = context.cache.write().await;
            cache.insert(cache_key.clone(), cacheable);
            cache.insert(
                NodeDBConnection::refresh_cache_key(&cache_key),
                refresh_hook,
            );
            drop(cache);

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
