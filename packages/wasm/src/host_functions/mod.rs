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
        self.store_cache.read().get(store_ref).cloned()
    }

    pub fn register_store(&self, store_ref: &str, store: FlowLikeStore) {
        self.store_cache
            .write()
            .insert(store_ref.to_string(), store);
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

/// Model context for WASM modules — provides model access including auth tokens.
pub struct ModelContext {
    pub app_state: Arc<flow_like::state::FlowLikeState>,
    pub token: Option<String>,
}

impl std::fmt::Debug for ModelContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelContext").finish()
    }
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
    /// Active WebSocket connections (session_id -> connection)
    pub ws_connections: Arc<tokio::sync::Mutex<HashMap<String, WsConnection>>>,
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
            ws_connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
