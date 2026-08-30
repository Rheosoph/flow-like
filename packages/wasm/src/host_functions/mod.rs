//! Host functions for WASM modules
//!
//! These functions are imported by WASM modules to interact with the Flow-Like runtime.

pub mod auth;
pub mod cache;
pub mod http;
pub mod linker;
pub mod logging;
pub mod metadata;
pub mod pins;
pub mod schema;
pub mod storage;
pub mod streaming;
pub mod variables;
pub mod websocket;

use crate::host_functions::storage::StorageFlowPath;
use crate::limits::WasmCapabilities;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::path::Path;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub use linker::register_host_functions;
pub use websocket::WsConnection;

/// Storage context for WASM modules — resolves stores server-side without exposing credentials.
pub struct StorageContext {
    pub stores: flow_like::state::FlowLikeStores,
    pub store_cache: RwLock<HashMap<String, FlowLikeStore>>,
    /// Online desktop runs provide a credential-backed content store. Native
    /// FlowPath nodes use this as the primary store and keep the configured app
    /// store as a cache layer; WASM storage must mirror that behavior.
    pub credentials_store: Option<FlowLikeStore>,
    pub app_id: String,
    pub board_dir: Path,
    pub board_id: String,
    pub node_id: String,
    pub sub: String,
}

impl std::fmt::Debug for StorageContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageContext")
            .field("app_id", &self.app_id)
            .field("board_id", &self.board_id)
            .field("node_id", &self.node_id)
            .finish()
    }
}

impl StorageContext {
    pub fn resolve_store(&self, store_ref: &str) -> Option<FlowLikeStore> {
        if let Some(store) = self.store_cache.read().get(store_ref).cloned() {
            return Some(store);
        }

        // Foreign store_ref from native catalog nodes (e.g. "dirs__upload_..." without
        // "wasm_" prefix). Match the pattern and auto-register the equivalent store.
        let store = self.resolve_foreign_store(store_ref)?;
        self.register_store(store_ref, store.clone());
        Some(store)
    }

    fn resolve_foreign_store(&self, store_ref: &str) -> Option<FlowLikeStore> {
        let key = store_ref.strip_prefix("wasm_").unwrap_or(store_ref);

        if let Some(dir_type) = Self::dir_type_from_store_ref(key, "cache_dirs__") {
            let store = self.backing_store_for_dir(dir_type);
            if store.is_none() {
                tracing::warn!(
                    "[wasm] resolve_foreign_store: backing store is None for {store_ref}"
                );
            }
            return store;
        }

        if let Some(dir_type) = Self::dir_type_from_store_ref(key, "dirs__") {
            let store = self.primary_store_for_dir(dir_type);
            if store.is_none() {
                tracing::warn!(
                    "[wasm] resolve_foreign_store: primary store is None for {store_ref}"
                );
            }
            return store;
        }

        tracing::warn!("[wasm] resolve_foreign_store: no pattern matched for {store_ref}");
        None
    }

    pub fn register_store(&self, store_ref: &str, store: FlowLikeStore) {
        self.store_cache
            .write()
            .insert(store_ref.to_string(), store);
    }

    pub fn dir_flow_path(&self, dir_type: &str, dir: Path) -> Option<StorageFlowPath> {
        let store_ref = format!("dirs__{dir_type}_{}", dir.as_ref());
        let primary_store = self.primary_store_for_dir(dir_type)?;
        self.register_store(&store_ref, primary_store);

        let cache_store_ref = self.cache_store_for_dir(dir_type).map(|cache_store| {
            let cache_store_ref = format!("cache_dirs__{dir_type}_{}", dir.as_ref());
            self.register_store(&cache_store_ref, cache_store);
            cache_store_ref
        });

        Some(StorageFlowPath {
            path: dir.as_ref().to_string(),
            store_ref,
            cache_store_ref,
        })
    }

    fn dir_type_from_store_ref(key: &str, prefix: &str) -> Option<&'static str> {
        for dir_type in ["upload", "storage", "cache", "user"] {
            let marker = format!("{prefix}{dir_type}_");
            if key.starts_with(&marker) {
                return Some(dir_type);
            }
        }
        None
    }

    fn backing_store_for_dir(&self, dir_type: &str) -> Option<FlowLikeStore> {
        match dir_type {
            "upload" | "storage" => self.stores.app_storage_store.clone(),
            "cache" => self.stores.temporary_store.clone(),
            "user" => self.stores.user_store.clone(),
            _ => None,
        }
    }

    fn primary_store_for_dir(&self, dir_type: &str) -> Option<FlowLikeStore> {
        self.credentials_store
            .clone()
            .or_else(|| self.backing_store_for_dir(dir_type))
    }

    fn cache_store_for_dir(&self, dir_type: &str) -> Option<FlowLikeStore> {
        if self.credentials_store.is_some() {
            return self.backing_store_for_dir(dir_type);
        }
        None
    }

    pub fn get_storage_dir(&self, node: bool) -> Path {
        let base = self.board_dir.child("storage");
        if node {
            base.child(self.node_id.clone())
        } else {
            base
        }
    }

    pub fn get_upload_dir(&self) -> Path {
        self.board_dir.child("upload")
    }

    pub fn get_cache_dir(&self, node: bool, user: bool) -> Path {
        let mut base = Path::from("tmp");
        if user {
            base = base.child("user").child(self.sub.clone());
        } else {
            base = base.child("global");
        }
        base = base.child("apps").child(self.app_id.clone());
        if node {
            base.child(self.node_id.clone())
        } else {
            base
        }
    }

    pub fn get_user_dir(&self, node: bool) -> Path {
        let base = Path::from("users")
            .child(self.sub.clone())
            .child("apps")
            .child(self.app_id.clone());
        if node {
            base.child(self.node_id.clone())
        } else {
            base
        }
    }
}

/// Shared store of resolved model handles, keyed by the model's cache key.
pub type ModelCacheHandle = Arc<
    flow_like_types::sync::RwLock<ahash::AHashMap<String, Arc<dyn flow_like_types::Cacheable>>>,
>;

/// Model context for WASM modules — provides model access including auth tokens.
#[derive(Clone)]
pub struct ModelContext {
    pub app_state: Arc<flow_like::state::FlowLikeState>,
    pub token: Option<String>,
    pub cache: Option<ModelCacheHandle>,
}

impl std::fmt::Debug for ModelContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelContext").finish()
    }
}

#[cfg(feature = "model")]
pub(crate) async fn resolve_cached_text_embedding_model(
    context: &ModelContext,
    model_json: &str,
) -> Option<Arc<dyn flow_like_model_provider::embedding::EmbeddingModelLogic>> {
    #[derive(serde::Deserialize)]
    struct CachedEmbeddingHandle {
        cache_key: String,
    }

    let handle: CachedEmbeddingHandle = serde_json::from_str(model_json).ok()?;
    let cache = context.cache.as_ref()?;
    let cached = cache.read().await.get(&handle.cache_key).cloned()?;
    let cached = cached
        .as_any()
        .downcast_ref::<flow_like_catalog_llm::embedding::CachedEmbeddingModelObject>()?;
    cached.text_model.clone()
}

/// Host state accessible from host functions
#[derive(Debug)]
pub struct HostState {
    /// Granted capabilities
    pub capabilities: WasmCapabilities,
    /// Output values set by WASM
    pub outputs: RwLock<HashMap<String, Value>>,
    /// Execution pins to activate
    pub exec_pins: RwLock<Vec<String>>,
    /// Log entries from WASM
    pub logs: RwLock<Vec<LogEntry>>,
    /// Error message if any
    pub error: RwLock<Option<String>>,
    /// Result buffer for returning data to WASM
    pub result_buffer: RwLock<Vec<u8>>,
    /// Input values (set before execution)
    pub inputs: RwLock<HashMap<String, Value>>,
    /// Variables (shared with execution context)
    pub variables: RwLock<HashMap<String, Value>>,
    /// Cache entries
    pub cache: RwLock<HashMap<String, Value>>,
    /// OAuth tokens (provider_id -> token)
    pub oauth_tokens: RwLock<HashMap<String, OAuthTokenData>>,
    /// Execution metadata
    pub metadata: ExecutionMetadata,
    /// Stream events to send
    pub stream_events: RwLock<Vec<StreamEvent>>,
    /// Storage context for server-side store resolution
    pub storage_context: Option<StorageContext>,
    /// Model context for server-side model access
    pub model_context: Option<ModelContext>,
    /// Usage attribution forwarded to hosted model APIs. Offline app runs keep
    /// the app ID unset while retaining their run ID.
    pub model_usage_context: Option<flow_like::models::llm::ModelUsageContext>,
    /// Active WebSocket connections (session_id -> connection)
    pub ws_connections: Arc<tokio::sync::Mutex<HashMap<String, WsConnection>>>,
    /// In-flight chunked writes (write_id -> buffer)
    pub pending_writes: RwLock<HashMap<String, storage::PendingWrite>>,
}

/// Log entry from WASM
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: u8,
    pub message: String,
    pub data: Option<Value>,
}

/// OAuth token data for WASM access
#[derive(Debug, Clone)]
pub struct OAuthTokenData {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: Option<i64>,
    pub refresh_token: Option<String>,
    pub scopes: Vec<String>,
}

/// Execution metadata accessible from WASM
#[derive(Debug, Clone, Default)]
pub struct ExecutionMetadata {
    pub node_id: String,
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
    pub user_id: String,
    pub stream_state: bool,
    pub log_level: u8,
    /// Where the enclosing flow runs, exposed to the guest for its own
    /// decisions. Informational only: enforcement reads
    /// `ComponentStoreData::environment`, which is stamped from the security
    /// config, because this struct derives `Default` and that default is the
    /// permissive `Local`.
    pub execution_environment: flow_like::flow::execution::ExecutionEnvironment,
}

/// Stream event from WASM
#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub event_type: String,
    pub data: Value,
}

impl HostState {
    pub fn new(capabilities: WasmCapabilities) -> Self {
        Self {
            capabilities,
            outputs: RwLock::new(HashMap::new()),
            exec_pins: RwLock::new(Vec::new()),
            logs: RwLock::new(Vec::new()),
            error: RwLock::new(None),
            result_buffer: RwLock::new(Vec::new()),
            inputs: RwLock::new(HashMap::new()),
            variables: RwLock::new(HashMap::new()),
            cache: RwLock::new(HashMap::new()),
            oauth_tokens: RwLock::new(HashMap::new()),
            metadata: ExecutionMetadata::default(),
            stream_events: RwLock::new(Vec::new()),
            storage_context: None,
            model_context: None,
            model_usage_context: None,
            ws_connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            pending_writes: RwLock::new(HashMap::new()),
        }
    }

    /// Check if a capability is granted
    pub fn has_capability(&self, cap: WasmCapabilities) -> bool {
        self.capabilities.has(cap)
    }

    /// Set input values before execution
    pub fn set_inputs(&self, inputs: HashMap<String, Value>) {
        *self.inputs.write() = inputs;
    }

    /// Get an input value
    pub fn get_input(&self, name: &str) -> Option<Value> {
        self.inputs.read().get(name).cloned()
    }

    /// Set an output value
    pub fn set_output(&self, name: &str, value: Value) {
        self.outputs.write().insert(name.to_string(), value);
    }

    /// Get all outputs
    pub fn get_outputs(&self) -> HashMap<String, Value> {
        self.outputs.read().clone()
    }

    /// Activate an execution pin
    pub fn activate_exec(&self, name: &str) {
        self.exec_pins.write().push(name.to_string());
    }

    /// Get activated execution pins
    pub fn get_activated_exec_pins(&self) -> Vec<String> {
        self.exec_pins.read().clone()
    }

    /// Add a log entry
    pub fn log(&self, level: u8, message: String, data: Option<Value>) {
        self.logs.write().push(LogEntry {
            level,
            message,
            data,
        });
    }

    /// Get all log entries
    pub fn get_logs(&self) -> Vec<LogEntry> {
        self.logs.read().clone()
    }

    /// Set error message
    pub fn set_error(&self, error: String) {
        *self.error.write() = Some(error);
    }

    /// Get error message
    pub fn get_error(&self) -> Option<String> {
        self.error.read().clone()
    }

    /// Store result in buffer and return packed pointer+length
    pub fn store_result(&self, data: &[u8]) -> (u32, u32) {
        let mut buffer = self.result_buffer.write();
        let ptr = buffer.len() as u32;
        buffer.extend_from_slice(data);
        (ptr, data.len() as u32)
    }

    /// Set metadata
    pub fn set_metadata(&mut self, metadata: ExecutionMetadata) {
        self.metadata = metadata;
    }

    /// Add stream event
    pub fn add_stream_event(&self, event_type: String, data: Value) {
        self.stream_events
            .write()
            .push(StreamEvent { event_type, data });
    }

    /// Get and clear stream events
    pub fn take_stream_events(&self) -> Vec<StreamEvent> {
        std::mem::take(&mut *self.stream_events.write())
    }

    /// Get a variable
    pub fn get_variable(&self, name: &str) -> Option<Value> {
        self.variables.read().get(name).cloned()
    }

    /// Set a variable
    pub fn set_variable(&self, name: &str, value: Value) {
        self.variables.write().insert(name.to_string(), value);
    }

    /// Stream an event to the client
    pub fn stream_event(&self, event_type: &str, data: &str) {
        let value: Value = serde_json::from_str(data).unwrap_or(Value::String(data.to_string()));
        self.add_stream_event(event_type.to_string(), value);
    }

    /// Reset state for reuse
    pub fn reset(&self) {
        self.outputs.write().clear();
        self.exec_pins.write().clear();
        self.logs.write().clear();
        *self.error.write() = None;
        self.result_buffer.write().clear();
        self.stream_events.write().clear();
    }
}
