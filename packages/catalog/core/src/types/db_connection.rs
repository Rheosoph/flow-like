#[cfg(feature = "execute")]
use flow_like::flow::execution::{InternalRun, context::ExecutionContext};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::{
    VectorStore,
    buffered::{
        BufferedVectorStore, BufferedWriteError, BufferedWriteFailure, BufferedWriteKind,
        BufferedWriteOrigin,
    },
    lancedb::LanceDBVectorStore,
};
#[cfg(feature = "execute")]
use flow_like_types::{
    Cacheable, Value, anyhow, async_trait,
    sync::RwLock,
    tokio::sync::{RwLockReadGuard, RwLockWriteGuard},
};
use flow_like_types::{
    JsonSchema,
    json::{Deserialize, Serialize},
};
#[cfg(feature = "execute")]
use std::collections::BTreeMap;
#[cfg(feature = "execute")]
use std::sync::Arc;

#[cfg(all(test, feature = "execute"))]
mod tests;

#[derive(Default, Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct NodeDBConnection {
    pub cache_key: String,
}

#[cfg(feature = "execute")]
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
#[cfg(feature = "execute")]
#[async_trait]
pub trait CachedDBRefresher: Send + Sync {
    /// Source handles must use `load_unflushed_with_generation` here so refreshing
    /// a writer cannot recursively flush that writer through another handle.
    async fn refresh(&self, context: &ExecutionContext) -> flow_like_types::Result<()>;

    /// Monotonically increasing generation of the credential-bearing store.
    /// Consumers that retain a derived handle (for example a DataFusion table
    /// provider) can rebuild it only when a refresh actually swapped stores.
    fn generation(&self) -> u64 {
        0
    }
}

#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct CachedDBRefreshHook {
    refresher: Arc<dyn CachedDBRefresher>,
}

#[cfg(feature = "execute")]
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

#[cfg(feature = "execute")]
impl Cacheable for CachedDBRefreshHook {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
impl Cacheable for CachedDB {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
impl CachedDB {
    /// A durable local receipt does not imply that replay reached the cloud.
    pub async fn last_write_receipt(
        &self,
    ) -> Option<flow_like_storage::databases::vector::lancedb::LocalWriteReceipt> {
        self.db.read().await.inner().last_write_receipt()
    }

    pub fn write_origin(context: &ExecutionContext) -> BufferedWriteOrigin {
        BufferedWriteOrigin::new(context.id.clone(), Some(context.trace.id.clone()))
    }

    pub async fn insert_from(
        &self,
        context: &ExecutionContext,
        items: Vec<Value>,
    ) -> flow_like_types::Result<()> {
        self.insert_from_with_receipt(context, items)
            .await
            .map(|_| ())
    }

    pub async fn insert_from_with_receipt(
        &self,
        context: &ExecutionContext,
        items: Vec<Value>,
    ) -> flow_like_types::Result<
        Option<flow_like_storage::databases::vector::lancedb::LocalWriteReceipt>,
    > {
        let mut db = self.db.write().await;
        db.insert_with_origin(items, Self::write_origin(context))
            .await?;
        Ok(db.inner().last_write_receipt())
    }

    pub async fn upsert_from(
        &self,
        context: &ExecutionContext,
        items: Vec<Value>,
        id_field: String,
    ) -> flow_like_types::Result<()> {
        self.upsert_from_with_receipt(context, items, id_field)
            .await
            .map(|_| ())
    }

    pub async fn upsert_from_with_receipt(
        &self,
        context: &ExecutionContext,
        items: Vec<Value>,
        id_field: String,
    ) -> flow_like_types::Result<
        Option<flow_like_storage::databases::vector::lancedb::LocalWriteReceipt>,
    > {
        let mut db = self.db.write().await;
        db.upsert_with_origin(items, id_field, Self::write_origin(context))
            .await?;
        Ok(db.inner().last_write_receipt())
    }

    /// Flushes buffered writes and keeps the lock. Parallel branches share the
    /// store, so releasing it before the caller's operation would let another
    /// branch queue a write in between.
    pub async fn flushed_write(
        &self,
    ) -> flow_like_types::Result<RwLockWriteGuard<'_, BufferedVectorStore<LanceDBVectorStore>>>
    {
        let mut db = self.db.write().await;
        if db.is_dirty() {
            db.flush().await?;
        }
        if let Some(report) = db.write_failure_report() {
            return Err(anyhow!(report));
        }
        Ok(db)
    }

    pub async fn flushed_read(
        &self,
    ) -> flow_like_types::Result<RwLockReadGuard<'_, BufferedVectorStore<LanceDBVectorStore>>>
    {
        Ok(self.flushed_write().await?.downgrade())
    }

    /// Only for callers that do not touch the store afterwards; everything
    /// else must hold the guard from `flushed_read` or `flushed_write`.
    pub async fn ensure_flushed(&self) -> flow_like_types::Result<()> {
        self.flushed_write().await.map(drop)
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

#[cfg(feature = "execute")]
impl NodeDBConnection {
    pub fn refresh_cache_key(cache_key: &str) -> String {
        format!("{cache_key}::refresh")
    }

    pub async fn load(&self, context: &mut ExecutionContext) -> flow_like_types::Result<CachedDB> {
        Ok(self.load_with_generation(context).await?.0)
    }

    /// Loads the database and returns its freshness generation. Before refreshing
    /// an unpinned read-only handle, flushes writers for the same database, table
    /// and branch so reads include writes buffered earlier in this run.
    pub async fn load_with_generation(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<(CachedDB, u64)> {
        self.load_inner(context, true).await
    }

    /// Inspects committed state without flushing pending writes. Used by reference
    /// metadata and credential refresh hooks that load a view's source handle.
    pub async fn load_unflushed_with_generation(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<(CachedDB, u64)> {
        self.load_inner(context, false).await
    }

    async fn refresh(&self, context: &ExecutionContext) -> flow_like_types::Result<u64> {
        let refresh_key = Self::refresh_cache_key(&self.cache_key);
        let refresh_hook = context.cache.read().await.get(&refresh_key).cloned();
        if let Some(refresh_hook) = refresh_hook {
            let refresh_hook = refresh_hook
                .as_any()
                .downcast_ref::<CachedDBRefreshHook>()
                .ok_or_else(|| {
                    flow_like_types::anyhow!("Database refresh cache has an unexpected type")
                })?
                .clone();
            refresh_hook.refresh(context).await?;
            Ok(refresh_hook.generation())
        } else {
            Ok(0)
        }
    }

    async fn load_inner(
        &self,
        context: &ExecutionContext,
        flush_writers: bool,
    ) -> flow_like_types::Result<(CachedDB, u64)> {
        let generation = self.refresh(context).await?;
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
        if flush_writers {
            Self::flush_related_writers(context, db).await?;
        }
        let store = db.db.write().await;
        let inner = store.inner();
        let selector = inner.selector();
        let mut generation = generation.wrapping_add(inner.mutation_generation());
        if selector.read_only && !selector.is_pinned() && !inner.is_durably_managed() {
            // Read-only is a capability restriction, not a version pin. A separate
            // writer advances its own Lance handle, so this cached reader must
            // resolve latest again before a node or a retained SQL provider reads it.
            let table = inner.raw().await?;
            table.checkout_latest().await?;
            generation = generation.wrapping_add(table.version().await?);
        }
        drop(store);
        Ok((db.clone(), generation))
    }

    async fn flush_related_writers(
        context: &ExecutionContext,
        reader: &CachedDB,
    ) -> flow_like_types::Result<()> {
        let (uri, table, branch) = {
            let store = reader.db.read().await;
            let inner = store.inner();
            let selector = inner.selector();
            if !selector.read_only || selector.is_pinned() || inner.is_durably_managed() {
                return Ok(());
            }
            (
                inner.connection()?.uri().to_owned(),
                inner.table_name().to_owned(),
                selector.branch,
            )
        };
        // Release the run cache and reader locks before touching another handle.
        // Refresh hooks may themselves load the source of a derived view.
        let writers = context
            .cache
            .read()
            .await
            .iter()
            .filter_map(|(key, cached)| {
                cached
                    .as_any()
                    .downcast_ref::<CachedDB>()
                    .filter(|cached| !Arc::ptr_eq(&cached.db, &reader.db))
                    .map(|cached| {
                        (
                            NodeDBConnection {
                                cache_key: key.clone(),
                            },
                            cached.clone(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        for (connection, cached) in writers {
            let matches = {
                let store = cached.db.read().await;
                let inner = store.inner();
                let selector = inner.selector();
                !selector.is_read_only()
                    && !inner.is_durably_managed()
                    && inner.table_name() == table
                    && selector.branch == branch
                    && inner.connection()?.uri() == uri
                    && (store.is_dirty() || store.has_write_failures())
            };
            if matches {
                connection.refresh(context).await?;
                cached.ensure_flushed().await?;
            }
        }
        Ok(())
    }
}
