#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_storage::databases::graph::lancegraph::LanceGraphStore;
#[cfg(feature = "execute")]
use flow_like_types::Cacheable;
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Cached graph store instance, stored in the execution context cache.
#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct CachedGraphStore {
    pub store: Arc<LanceGraphStore>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedGraphStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(feature = "execute")]
pub async fn load_graph_store(
    context: &ExecutionContext,
    cache_key: &str,
) -> flow_like_types::Result<Arc<LanceGraphStore>> {
    let cached =
        context
            .cache
            .read()
            .await
            .get(cache_key)
            .cloned()
            .ok_or(flow_like_types::anyhow!(
                "Graph store not found in cache (key: {})",
                cache_key
            ))?;
    let store =
        cached
            .as_any()
            .downcast_ref::<CachedGraphStore>()
            .ok_or(flow_like_types::anyhow!(
                "Could not downcast cached value to CachedGraphStore"
            ))?;
    Ok(store.store.clone())
}
