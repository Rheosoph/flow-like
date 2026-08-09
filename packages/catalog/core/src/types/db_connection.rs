use flow_like::flow::execution::{InternalRun, context::ExecutionContext};
use flow_like_storage::databases::vector::{
    VectorStore,
    buffered::{
        BufferedVectorStore, BufferedWriteError, BufferedWriteFailure, BufferedWriteKind,
        BufferedWriteOrigin,
    },
    lancedb::LanceDBVectorStore,
};
use flow_like_types::{
    Cacheable, JsonSchema, Value, anyhow, async_trait,
    json::{Deserialize, Serialize},
    sync::RwLock,
};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default, Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct NodeDBConnection {
    pub cache_key: String,
}

#[derive(Clone)]
pub struct CachedDB {
    pub db: Arc<RwLock<BufferedVectorStore<LanceDBVectorStore>>>,
}

/// Optional hook associated with a [`NodeDBConnection`] cache entry.
///
/// Most databases have process-lifetime credentials and do not need one. A
/// remote database can install a hook under the companion cache key so every
/// consumer gets a cheap freshness check before loading the cached store. The
/// hook owns any single-flight state needed to replace an expiring store.
#[async_trait]
pub trait CachedDBRefresher: Send + Sync {
    async fn refresh(&self, context: &ExecutionContext) -> flow_like_types::Result<()>;

    /// Monotonically increasing generation of the credential-bearing store.
    /// Consumers that retain a derived handle (for example a DataFusion table
    /// provider) can rebuild it only when a refresh actually swapped stores.
    fn generation(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
pub struct CachedDBRefreshHook {
    refresher: Arc<dyn CachedDBRefresher>,
}

impl CachedDBRefreshHook {
    pub fn new(refresher: Arc<dyn CachedDBRefresher>) -> Self {
        Self { refresher }
    }

    async fn refresh(&self, context: &ExecutionContext) -> flow_like_types::Result<()> {
        self.refresher.refresh(context).await
    }

    fn generation(&self) -> u64 {
        self.refresher.generation()
    }
}

impl Cacheable for CachedDBRefreshHook {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Cacheable for CachedDB {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl CachedDB {
    pub fn write_origin(context: &ExecutionContext) -> BufferedWriteOrigin {
        BufferedWriteOrigin::new(context.id.clone(), Some(context.trace.id.clone()))
    }

    pub async fn insert_from(
        &self,
        context: &ExecutionContext,
        items: Vec<Value>,
    ) -> flow_like_types::Result<()> {
        self.db
            .write()
            .await
            .insert_with_origin(items, Self::write_origin(context))
            .await
    }

    pub async fn upsert_from(
        &self,
        context: &ExecutionContext,
        items: Vec<Value>,
        id_field: String,
    ) -> flow_like_types::Result<()> {
        self.db
            .write()
            .await
            .upsert_with_origin(items, id_field, Self::write_origin(context))
            .await
    }

    pub async fn ensure_flushed(&self) -> flow_like_types::Result<()> {
        let mut db = self.db.write().await;
        if db.is_dirty() {
            db.flush().await?;
        }
        if let Some(report) = db.write_failure_report() {
            return Err(anyhow!(report));
        }
        Ok(())
    }

    pub async fn has_buffered_writes(&self) -> bool {
        self.db.read().await.is_dirty()
    }

    async fn log_write_failures(
        run: &InternalRun,
        fallback_origin: &BufferedWriteOrigin,
        failures: Vec<BufferedWriteFailure>,
    ) {
        let mut grouped =
            BTreeMap::<(BufferedWriteOrigin, BufferedWriteKind, String), usize>::new();
        for failure in failures {
            let origin = failure.origin.unwrap_or_else(|| fallback_origin.clone());
            *grouped
                .entry((origin, failure.operation, failure.error))
                .or_default() += 1;
        }

        for ((origin, operation, error), count) in grouped {
            run.log_node_error(
                origin.node_id.as_ref(),
                origin.operation_id,
                &format!(
                    "Database {operation} failed: {count} buffered row(s) were not persisted: {error}"
                ),
            )
            .await;
        }
    }

    /// Logs every terminal buffered-write failure against the node invocation
    /// that queued the row, then returns an error so the run cannot report a
    /// successful completion.
    pub async fn flush_on_completion(
        &self,
        run: &InternalRun,
        fallback_origin: &BufferedWriteOrigin,
    ) -> flow_like_types::Result<()> {
        let (flush_error, failures) = {
            let mut db = self.db.write().await;
            let flush_error = if db.is_dirty() {
                db.flush().await.err()
            } else {
                None
            };
            let failures = db.take_write_failures();
            (flush_error, failures)
        };

        if failures.is_empty() {
            if let Some(error) = flush_error {
                run.log_node_error(
                    fallback_origin.node_id.as_ref(),
                    fallback_origin.operation_id.clone(),
                    &format!("Database completion flush failed: {error:#}"),
                )
                .await;
                return Err(error);
            }
            return Ok(());
        }

        let report = BufferedWriteError::new(failures.clone());
        Self::log_write_failures(run, fallback_origin, failures).await;

        Err(flush_error.unwrap_or_else(|| anyhow!(report)))
    }

    /// Logs a prerequisite failure (for example credential refresh) against
    /// every node invocation that still has buffered rows at risk.
    pub async fn log_pending_write_error(
        &self,
        run: &InternalRun,
        fallback_origin: &BufferedWriteOrigin,
        message: &str,
    ) {
        let (origins, retained_failures, has_unattributed_writes) = {
            let mut db = self.db.write().await;
            let origins = db.pending_write_origins();
            let has_unattributed_writes = db.has_unattributed_pending_writes();
            let retained_failures = db.take_write_failures();
            (origins, retained_failures, has_unattributed_writes)
        };

        Self::log_write_failures(run, fallback_origin, retained_failures).await;

        if has_unattributed_writes {
            run.log_node_error(
                fallback_origin.node_id.as_ref(),
                fallback_origin.operation_id.clone(),
                message,
            )
            .await;
        }

        for origin in origins {
            run.log_node_error(origin.node_id.as_ref(), origin.operation_id, message)
                .await;
        }
    }
}

impl NodeDBConnection {
    pub fn refresh_cache_key(cache_key: &str) -> String {
        format!("{cache_key}::refresh")
    }

    pub async fn load(&self, context: &mut ExecutionContext) -> flow_like_types::Result<CachedDB> {
        Ok(self.load_with_generation(context).await?.0)
    }

    /// Loads the database and returns the current refresh generation. Local
    /// databases always use generation zero.
    pub async fn load_with_generation(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<(CachedDB, u64)> {
        let refresh_key = Self::refresh_cache_key(&self.cache_key);
        let refresh_hook = context.cache.read().await.get(&refresh_key).cloned();
        let generation = if let Some(refresh_hook) = refresh_hook {
            let refresh_hook = refresh_hook
                .as_any()
                .downcast_ref::<CachedDBRefreshHook>()
                .ok_or_else(|| {
                    flow_like_types::anyhow!("Database refresh cache has an unexpected type")
                })?
                .clone();
            refresh_hook.refresh(context).await?;
            refresh_hook.generation()
        } else {
            0
        };

        let cached = context
            .cache
            .read()
            .await
            .get(self.cache_key.as_str())
            .cloned()
            .ok_or(flow_like_types::anyhow!("No cache found"))?;
        let db = cached
            .as_any()
            .downcast_ref::<CachedDB>()
            .ok_or(flow_like_types::anyhow!("Could not downcast"))?;
        Ok((db.clone(), generation))
    }
}
