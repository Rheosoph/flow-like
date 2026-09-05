#[cfg(feature = "execute")]
use flow_like::flow::execution::ExecutionEnvironment;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::datafusion::common::TableReference;
#[cfg(feature = "execute")]
use flow_like_storage::datafusion::execution::object_store::{
    DefaultObjectStoreRegistry, ObjectStoreRegistry,
};
#[cfg(feature = "execute")]
use flow_like_storage::datafusion::execution::runtime_env::{RuntimeEnv, RuntimeEnvBuilder};
#[cfg(all(feature = "execute", feature = "federation"))]
use flow_like_storage::datafusion::execution::session_state::SessionStateBuilder;
#[cfg(feature = "execute")]
use flow_like_storage::datafusion::prelude::{SessionConfig, SessionContext};
#[cfg(all(feature = "execute", feature = "federation"))]
use flow_like_storage::datafusion_federation::{FederatedQueryPlanner, default_optimizer_rules};
#[cfg(feature = "execute")]
use flow_like_storage::num_cpus;
use flow_like_types::JsonSchema;
#[cfg(feature = "execute")]
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::reqwest::Url;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, sync::Mutex};
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::{collections::HashMap, sync::Arc};

#[derive(Default, Serialize, Deserialize, JsonSchema, Clone)]
pub struct DataFusionSession {
    pub cache_key: String,
}

/// A table registration whose expensive work — file downloads, parsing, schema
/// inference, connection setup — is postponed until a node actually queries the engine.
///
/// Mount nodes enqueue one of these via [`CachedDataFusionSession::defer_mount`] instead
/// of registering immediately; [`DataFusionSession::load`] applies the queue before any
/// consumer touches `ctx`. A cached query that never loads the session therefore never
/// pays for the mounts either.
#[cfg(feature = "execute")]
#[async_trait]
pub trait DeferredMount: Send + Sync {
    /// Short human-readable description used in logs and error messages, e.g.
    /// "Excel workbook 'sales.xlsx'".
    fn describe(&self) -> String;

    /// Identity used to collapse repeats: deferring a mount evicts any queued mount
    /// with the same key (a mount node re-run in a loop replaces its earlier self
    /// instead of growing the queue and colliding at registration time). `None` never
    /// dedupes.
    fn dedupe_key(&self) -> Option<String> {
        None
    }

    /// Perform the registration. A failed materialization leaves the mount queued for
    /// the next consumer, so implementations must tolerate running again — deregister
    /// the target table name before registering it.
    async fn mount(
        &self,
        session: &CachedDataFusionSession,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()>;
}

#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct CachedDataFusionSession {
    pub ctx: Arc<SessionContext>,
    lance_tables: Arc<Mutex<HashMap<String, LanceTableRegistration>>>,
    pending_mounts: Arc<Mutex<Vec<Arc<dyn DeferredMount>>>>,
}

#[cfg(feature = "execute")]
#[derive(Clone)]
struct LanceTableRegistration {
    database: NodeDBConnection,
    generation: u64,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedDataFusionSession {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
impl DataFusionSession {
    /// Load the session and make it fully queryable: queued deferred mounts are applied
    /// and Lance registrations refreshed. Every node that reads from `ctx` must use
    /// this.
    pub async fn load(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<CachedDataFusionSession> {
        let session = self.load_lazy(context).await?;
        session.materialize(context).await?;
        session.refresh_lance_tables(context).await?;
        Ok(session)
    }

    /// Load the session handle without applying queued mounts. For mount nodes only:
    /// they add work to the session and must not force earlier deferred mounts to run.
    pub async fn load_lazy(
        &self,
        context: &ExecutionContext,
    ) -> flow_like_types::Result<CachedDataFusionSession> {
        let cached = context
            .cache
            .read()
            .await
            .get(self.cache_key.as_str())
            .cloned()
            .ok_or(flow_like_types::anyhow!(
                "DataFusion session not found in cache"
            ))?;
        let session = cached
            .as_any()
            .downcast_ref::<CachedDataFusionSession>()
            .ok_or(flow_like_types::anyhow!(
                "Could not downcast to DataFusion session"
            ))?;
        Ok(session.clone())
    }
}

#[cfg(feature = "execute")]
impl CachedDataFusionSession {
    pub fn new(ctx: SessionContext) -> Self {
        Self {
            ctx: Arc::new(ctx),
            lance_tables: Arc::new(Mutex::new(HashMap::new())),
            pending_mounts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Queue a mount to run when the session is next materialized by a consumer. A
    /// queued mount with the same dedupe key is replaced, not duplicated.
    pub async fn defer_mount(&self, mount: Arc<dyn DeferredMount>) {
        let mut pending = self.pending_mounts.lock().await;
        if let Some(key) = mount.dedupe_key() {
            pending.retain(|queued| queued.dedupe_key().as_deref() != Some(key.as_str()));
        }
        pending.push(mount);
    }

    /// Apply queued mounts in the order they were deferred.
    ///
    /// The queue lock is held across the whole drain: two consumers materializing
    /// concurrently must not both run the same mount. A mount leaves the queue only
    /// once it has succeeded — a failed (or cancelled) one stays queued and is retried
    /// by the next consumer, so a transient error cannot silently cost the session a
    /// table. The failure itself propagates with the mount's description attached.
    pub async fn materialize(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut pending = self.pending_mounts.lock().await;
        while let Some(mount) = pending.first().cloned() {
            mount.mount(self, context).await.map_err(|err| {
                flow_like_types::anyhow!("Deferred mount of {} failed: {err}", mount.describe())
            })?;
            pending.remove(0);
        }
        Ok(())
    }

    /// Remembers a Lance registration so derived DataFusion providers can be
    /// replaced when a remote database rotates its scoped credentials.
    pub async fn track_lance_table(
        &self,
        table_name: String,
        database: NodeDBConnection,
        generation: u64,
    ) {
        self.lance_tables.lock().await.insert(
            table_name,
            LanceTableRegistration {
                database,
                generation,
            },
        );
    }

    async fn refresh_lance_tables(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let mut registrations = self.lance_tables.lock().await;
        for (table_name, registration) in registrations.iter_mut() {
            let (cached_db, generation) =
                registration.database.load_with_generation(context).await?;
            // Flush before the generation short-circuit: local databases never
            // rotate generations, but SQL run through this session — including
            // UPDATE/DELETE — must see the flow's own buffered writes (a flush
            // after a DELETE would otherwise resurrect the deleted rows).
            cached_db.ensure_flushed().await?;
            if generation == registration.generation {
                continue;
            }

            let db_guard = cached_db.db.read().await;
            let adapter = db_guard.inner().to_datafusion().await?;
            drop(db_guard);
            // The catalog rejects registering an existing name, so the stale provider
            // must be dropped before the rotated one can take its place.
            self.ctx
                .deregister_table(TableReference::bare(table_name.clone()))?;
            self.ctx
                .register_table(TableReference::bare(table_name.clone()), adapter)?;
            registration.generation = generation;
        }
        Ok(())
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
pub fn build_session_config(
    target_partitions: i64,
    batch_size: i64,
    repartition_joins: bool,
    repartition_aggregations: bool,
    repartition_sorts: bool,
    coalesce_batches: bool,
    parquet_pruning: bool,
    collect_statistics: bool,
) -> SessionConfig {
    let target_partitions = if target_partitions <= 0 {
        num_cpus::get()
    } else {
        target_partitions as usize
    };

    let batch_size = batch_size.max(1) as usize;

    SessionConfig::new()
        .with_target_partitions(target_partitions)
        .with_batch_size(batch_size)
        .with_repartition_joins(repartition_joins)
        .with_repartition_aggregations(repartition_aggregations)
        .with_repartition_sorts(repartition_sorts)
        .with_coalesce_batches(coalesce_batches)
        .with_collect_statistics(collect_statistics)
        .with_parquet_pruning(parquet_pruning)
        .with_parquet_bloom_filter_pruning(parquet_pruning)
        .with_parquet_page_index_pruning(parquet_pruning)
}

/// DataFusion's default object-store registry maps `file://` to the host
/// filesystem, so tenant- or model-authored SQL such as
/// `CREATE EXTERNAL TABLE … LOCATION 'file:///proc/self/environ'` or
/// `COPY … TO 'file:///…'` would read or write wherever the executor process
/// can. Server-side, only explicitly registered stores (`flowlike://`, mounted
/// object stores) exist; a `file://` URL then fails with DataFusion's own
/// "No suitable object store found" error.
#[cfg(feature = "execute")]
fn create_runtime_env(environment: ExecutionEnvironment) -> Arc<RuntimeEnv> {
    let registry = DefaultObjectStoreRegistry::new();
    if environment == ExecutionEnvironment::Server
        && let Ok(file_scheme) = Url::parse("file:///")
    {
        let _ = registry.deregister_store(&file_scheme);
    }
    RuntimeEnvBuilder::new()
        .with_object_store_registry(Arc::new(registry))
        .build_arc()
        .expect("RuntimeEnv without disk spill or memory limits cannot fail to build")
}

/// Every DataFusion session in this crate must be built through here so the
/// server-side `file://` restriction applies to all SQL entry points.
#[cfg(all(feature = "execute", feature = "federation"))]
pub fn create_session_context(
    config: SessionConfig,
    environment: ExecutionEnvironment,
) -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_config(config)
        .with_runtime_env(create_runtime_env(environment))
        .with_optimizer_rules(default_optimizer_rules())
        .with_query_planner(Arc::new(FederatedQueryPlanner::new()))
        .with_default_features()
        .build();

    SessionContext::new_with_state(state)
}

/// Every DataFusion session in this crate must be built through here so the
/// server-side `file://` restriction applies to all SQL entry points.
#[cfg(all(feature = "execute", not(feature = "federation")))]
pub fn create_session_context(
    config: SessionConfig,
    environment: ExecutionEnvironment,
) -> SessionContext {
    SessionContext::new_with_config_rt(config, create_runtime_env(environment))
}
