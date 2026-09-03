//! F2 replay isolation: an execution context built from a shadow-flagged run
//! wraps app storage, user store and app meta store read-only, so a write
//! fails loudly while the same write on a normal run succeeds. The temporary
//! and log stores stay writable — scratch and run recording are not app state.

use flow_like::a2ui::ElementCache;
use flow_like::flow::execution::context::ExecutionContextCache;
use flow_like::flow::execution::{
    DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD, DEFAULT_RUN_LOG_FLUSH_INTERVAL, ExecutionEnvironment,
    ExecutionMode, RunMeta,
};
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::memory::InMemory;
use flow_like_types::sync::RwLock;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

fn run_meta(shadow: bool) -> RunMeta {
    RunMeta {
        run_id: "run-1".to_string(),
        app_id: "app-1".to_string(),
        model_usage_app_id: None,
        board_id: "board-1".to_string(),
        board_dir: Path::from("apps").child("app-1"),
        sub: "user-1".to_string(),
        stream_state: false,
        environment: ExecutionEnvironment::Local,
        execution_mode: ExecutionMode::Sync,
        log_spill_threshold: DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD,
        log_flush_interval: DEFAULT_RUN_LOG_FLUSH_INTERVAL,
        nodes_executed: Arc::new(AtomicU64::new(0)),
        elements: Arc::new(RwLock::new(ElementCache::default())),
        shadow,
    }
}

fn state_with_memory_stores() -> Arc<FlowLikeState> {
    let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
    let config = FlowLikeConfig::with_default_store(store);
    Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ))
}

async fn cache_for(shadow: bool, state: &Arc<FlowLikeState>) -> ExecutionContextCache {
    ExecutionContextCache::from_meta(&run_meta(shadow), state, Arc::from("node-1")).await
}

#[tokio::test]
async fn shadow_context_cannot_write_app_storage_while_a_normal_one_can() {
    let state = state_with_memory_stores();
    let path = Path::from("apps").child("app-1").child("file.txt");

    let normal = cache_for(false, &state).await;
    normal
        .stores
        .app_storage_store
        .as_ref()
        .expect("app storage store is configured")
        .put(&path, b"live".to_vec())
        .await
        .expect("a normal run writes app storage");

    let shadow = cache_for(true, &state).await;
    for (name, store) in [
        ("app_storage_store", &shadow.stores.app_storage_store),
        ("user_store", &shadow.stores.user_store),
        ("app_meta_store", &shadow.stores.app_meta_store),
    ] {
        let error = store
            .as_ref()
            .unwrap_or_else(|| panic!("{name} is configured"))
            .put(&path, b"mutation".to_vec())
            .await
            .expect_err(&format!("a shadow run must not write {name}"));
        assert!(
            error
                .to_string()
                .contains("shadow runs cannot write app storage"),
            "{name} write must fail with the shadow message, got: {error}"
        );
    }
    assert!(shadow.shadow, "the context carries the flag for bypasses");

    // Reads still delegate: the object the normal run wrote is visible.
    let bytes = shadow
        .stores
        .app_storage_store
        .as_ref()
        .unwrap()
        .as_generic()
        .get(&path)
        .await
        .expect("shadow reads delegate to the wrapped store")
        .bytes()
        .await
        .expect("body is readable");
    assert_eq!(bytes.as_ref(), b"live");

    // Scratch and log stores stay writable so the run itself is recorded.
    for (name, store) in [
        ("temporary_store", &shadow.stores.temporary_store),
        ("log_store", &shadow.stores.log_store),
    ] {
        store
            .as_ref()
            .unwrap_or_else(|| panic!("{name} is configured"))
            .put(&Path::from("tmp").child("scratch.txt"), b"ok".to_vec())
            .await
            .unwrap_or_else(|e| panic!("{name} must stay writable for a shadow run: {e}"));
    }
}
