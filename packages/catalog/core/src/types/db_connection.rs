use flow_like::flow::execution::context::ExecutionContext;
use flow_like_storage::databases::vector::{
    VectorStore, buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
};
use flow_like_types::{
    Cacheable, JsonSchema, async_trait,
    json::{Deserialize, Serialize},
    sync::RwLock,
};
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
    pub async fn ensure_flushed(&self) -> flow_like_types::Result<()> {
        if self.db.read().await.is_dirty() {
            self.db.write().await.flush().await?;
        }
        Ok(())
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
