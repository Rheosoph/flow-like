use flow_like::{
    flow_like_storage::object_store::ObjectStore, state::FlowLikeState, utils::http::HTTPClient,
};
use flow_like_types::sync::Mutex;
use flow_like_wasm::client::RegistryClient;
use flow_like_wasm::{WasmConfig, WasmEngine};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

#[cfg(desktop)]
use crate::tray::TrayRuntimeState;
use crate::{event_bus::EventBus, profile::UserProfile, settings::Settings};

pub use crate::functions::recording::state::TauriRecordingState;

/// One tokenised snapshot of a local board, pinned to the revision it was built from.
///
/// `previous` keeps the last few revisions' snapshots alive so a webview that still holds one
/// of their segment tokens gets a node-level patch instead of the whole segment (see
/// `BoardSyncSnapshot::diff`), and so the next build reuses tokens incrementally.
pub struct LocalBoardSnapshot {
    pub updated_at: std::time::SystemTime,
    pub hash: Option<u64>,
    pub snapshot: Arc<flow_like::flow::board::sync::BoardSyncSnapshot>,
    pub previous: Vec<Arc<flow_like::flow::board::sync::BoardSyncSnapshot>>,
}

/// Revisions retained per board for patch bases: the current one plus this many earlier ones.
pub const LOCAL_BOARD_SNAPSHOT_HISTORY: usize = 3;

impl LocalBoardSnapshot {
    /// The segment carrying `token` in this or one of the retained earlier revisions.
    pub fn segment_by_token(
        &self,
        token: &str,
    ) -> Option<Arc<flow_like::flow::board::sync::SyncSegment>> {
        self.snapshot.segment_by_token(token).or_else(|| {
            self.previous
                .iter()
                .find_map(|snapshot| snapshot.segment_by_token(token))
        })
    }
}

/// Snapshots answering the webview's incremental `sync_board` IPC calls, keyed like the board
/// registry (`{board_id}` or `{board_id}-{maj}-{min}-{pat}`).
///
/// A snapshot is reused only while the board's `(updated_at, hash)` pair is unchanged; every
/// desktop mutation path (`execute_commands`, undo/redo, remote upsert) moves at least one of
/// them. Registry refreshes rewrite node definitions without touching either, so they clear the
/// map wholesale.
#[derive(Clone, Default)]
pub struct TauriBoardSyncState(
    pub Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<LocalBoardSnapshot>>>>,
);

impl TauriBoardSyncState {
    pub fn invalidate_all(app_handle: &AppHandle) {
        if let Some(state) = app_handle.try_state::<TauriBoardSyncState>() {
            state.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
    }
}

#[derive(Clone)]
pub struct TauriFlowLikeState(pub Arc<FlowLikeState>);
impl TauriFlowLikeState {
    #[inline]
    pub async fn construct(app_handle: &AppHandle) -> anyhow::Result<Arc<FlowLikeState>> {
        app_handle
            .try_state::<TauriFlowLikeState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| anyhow::anyhow!("Flow-Like State not found"))
    }

    #[inline]
    pub async fn http_client(app_handle: &AppHandle) -> anyhow::Result<Arc<HTTPClient>> {
        let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
        let http_client = flow_like_state.http_client.clone();
        Ok(http_client)
    }

    #[inline]
    pub async fn get_project_storage_store(
        app_handle: &AppHandle,
    ) -> anyhow::Result<Arc<dyn ObjectStore>> {
        let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
        let project_store = flow_like_state
            .config
            .read()
            .await
            .stores
            .app_storage_store
            .clone()
            .ok_or(anyhow::anyhow!("Project store not found"))?
            .as_generic();
        Ok(project_store)
    }

    #[inline]
    pub async fn get_project_meta_store(
        app_handle: &AppHandle,
    ) -> anyhow::Result<Arc<dyn ObjectStore>> {
        let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
        let project_store = flow_like_state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or(anyhow::anyhow!("Project store not found"))?
            .as_generic();
        Ok(project_store)
    }
}

pub struct TauriSettingsState(pub Arc<Mutex<Settings>>);
impl TauriSettingsState {
    #[inline]
    pub async fn construct(app_handle: &AppHandle) -> anyhow::Result<Arc<Mutex<Settings>>> {
        app_handle
            .try_state::<TauriSettingsState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| anyhow::anyhow!("Settings State not found"))
    }

    #[inline]
    pub async fn current_profile(app_handle: &AppHandle) -> anyhow::Result<UserProfile> {
        let settings = TauriSettingsState::construct(app_handle).await?;
        let settings = settings.lock().await;
        let current_profile = settings.get_current_profile()?;
        Ok(current_profile)
    }
}

pub struct TauriEventBusState(pub Arc<EventBus>);
impl TauriEventBusState {
    #[inline]
    #[allow(dead_code)] // accessor parity with the other Tauri*State types; current consumers use try_state directly
    pub fn construct(app_handle: &AppHandle) -> anyhow::Result<Arc<EventBus>> {
        app_handle
            .try_state::<TauriEventBusState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| anyhow::anyhow!("EventBus State not found"))
    }
}

pub struct TauriEventSinkManagerState(pub Arc<Mutex<crate::event_sink::EventSinkManager>>);
impl TauriEventSinkManagerState {
    #[inline]
    pub async fn construct(
        app_handle: &AppHandle,
    ) -> anyhow::Result<Arc<Mutex<crate::event_sink::EventSinkManager>>> {
        app_handle
            .try_state::<TauriEventSinkManagerState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| anyhow::anyhow!("EventSinkManager State not found"))
    }
}

pub struct TauriRegistryState(pub Arc<Mutex<Option<RegistryClient>>>);
impl TauriRegistryState {
    #[inline]
    pub async fn construct(
        app_handle: &AppHandle,
    ) -> anyhow::Result<Arc<Mutex<Option<RegistryClient>>>> {
        app_handle
            .try_state::<TauriRegistryState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| anyhow::anyhow!("Registry State not found"))
    }

    #[inline]
    pub async fn get_client(app_handle: &AppHandle) -> anyhow::Result<RegistryClient> {
        let state: Arc<Mutex<Option<RegistryClient>>> = Self::construct(app_handle).await?;
        let guard = state.lock().await;
        guard
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Registry client not initialized"))
    }
}

pub struct TauriWasmEngineState(pub Arc<WasmEngine>);
impl TauriWasmEngineState {
    #[inline]
    pub fn construct(app_handle: &AppHandle) -> anyhow::Result<Arc<WasmEngine>> {
        app_handle
            .try_state::<TauriWasmEngineState>()
            .map(|state| state.0.clone())
            .ok_or_else(|| anyhow::anyhow!("WasmEngine State not found"))
    }

    pub fn create_shared() -> anyhow::Result<Arc<WasmEngine>> {
        let config = WasmConfig::development()
            .with_cache_dir(flow_like::utils::cache::get_cache_dir().join("wasm"));

        let engine = WasmEngine::new(config)
            .map_err(|e| anyhow::anyhow!("Failed to create shared WasmEngine: {}", e))?;
        Ok(Arc::new(engine))
    }
}

#[cfg(desktop)]
pub struct TauriTrayState(pub Arc<Mutex<TrayRuntimeState>>);
