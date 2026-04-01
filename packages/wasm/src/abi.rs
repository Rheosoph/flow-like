//! ABI definitions for WASM nodes
//!
//! Defines the interface contract between Flow-Like runtime and WASM modules.

use flow_like::flow::node::NodePermission;
use serde::{Deserialize, Serialize};

/// Current ABI version - bump when making breaking changes
pub const WASM_ABI_VERSION: u32 = 1;

/// Module name for host function imports
pub const HOST_MODULE_NAME: &str = "flow_like";

/// Required exports that every WASM node module must provide
pub mod exports {
    /// Returns node definition as JSON
    /// Signature: () -> i64 (pointer << 32 | length)
    pub const GET_NODE: &str = "get_node";

    /// Execute node logic
    /// Signature: (context_ptr: i32, context_len: i32) -> i64 (result pointer << 32 | length, or negative error code)
    pub const RUN: &str = "run";

    /// Optional: Returns ABI version
    /// Signature: () -> i32
    pub const GET_ABI_VERSION: &str = "get_abi_version";

    /// Optional: Called when module is being unloaded
    /// Signature: () -> ()
    pub const ON_DROP: &str = "on_drop";

    /// Optional: Allocate memory for host to write into
    /// Signature: (size: i32) -> i32 (pointer)
    pub const ALLOC: &str = "alloc";

    /// Optional: Free previously allocated memory
    /// Signature: (ptr: i32, size: i32) -> ()
    pub const DEALLOC: &str = "dealloc";

    /// Optional: Get multiple node definitions (for multi-node modules)
    /// Signature: () -> i64 (pointer << 32 | length)
    pub const GET_NODES: &str = "get_nodes";
}

/// WASM ABI helper functions
pub struct WasmAbi;

impl WasmAbi {
    /// Pack pointer and length into i64 for return values
    /// High 32 bits: pointer, Low 32 bits: length
    #[inline]
    pub fn pack_ptr_len(ptr: u32, len: u32) -> i64 {
        ((ptr as i64) << 32) | (len as i64)
    }

    /// Unpack i64 into pointer and length
    #[inline]
    pub fn unpack_ptr_len(packed: i64) -> (u32, u32) {
        let ptr = (packed >> 32) as u32;
        let len = (packed & 0xFFFFFFFF) as u32;
        (ptr, len)
    }

    /// Check if packed value is an error code (negative)
    #[inline]
    pub fn is_error(packed: i64) -> bool {
        packed < 0
    }

    /// Get error code from packed value
    #[inline]
    pub fn get_error_code(packed: i64) -> i32 {
        packed as i32
    }
}

/// Node definition as returned by WASM module's get_node()
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmNodeDefinition {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub pins: Vec<WasmPinDefinition>,
    #[serde(default)]
    pub scores: Option<WasmNodeScores>,
    #[serde(default)]
    pub long_running: Option<bool>,
    #[serde(default)]
    pub docs: Option<String>,
    /// ABI version this module was built for
    #[serde(default)]
    pub abi_version: Option<u32>,
    /// Per-node permissions declared by the WASM module.
    /// Empty means the node needs no additional permissions.
    #[serde(default)]
    pub permissions: Vec<NodePermission>,
}

/// Pin definition for WASM nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmPinDefinition {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    /// "Input" or "Output"
    pub pin_type: String,
    /// "Execution", "String", "Integer", "Float", "Boolean", "Date", "PathBuf", "Struct", "Byte", "Generic"
    pub data_type: String,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    /// "Normal", "Array", "HashMap", "HashSet"
    #[serde(default)]
    pub value_type: Option<String>,
    /// JSON schema for Struct types
    #[serde(default)]
    pub schema: Option<String>,
    /// Valid values for enum-like pins
    #[serde(default)]
    pub valid_values: Option<Vec<String>>,
    /// Range for numeric pins (min, max)
    #[serde(default)]
    pub range: Option<(f64, f64)>,
    /// Step for numeric pins (slider increment)
    #[serde(default)]
    pub step: Option<f64>,
    /// Whether the pin value is sensitive (e.g. passwords, API keys)
    #[serde(default)]
    pub sensitive: Option<bool>,
    /// Enforce that the value matches the declared JSON schema
    #[serde(default)]
    pub enforce_schema: Option<bool>,
    /// Enforce that the generic value type matches at connection time
    #[serde(default)]
    pub enforce_generic_value_type: Option<bool>,
}

/// Node quality scores
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmNodeScores {
    #[serde(default)]
    pub privacy: u8,
    #[serde(default)]
    pub security: u8,
    #[serde(default)]
    pub performance: u8,
    #[serde(default)]
    pub governance: u8,
    #[serde(default)]
    pub reliability: u8,
    #[serde(default)]
    pub cost: u8,
}

/// Execution context passed to WASM run() function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmExecutionInput {
    /// Input pin values (name -> JSON value)
    pub inputs: serde_json::Map<String, serde_json::Value>,
    /// Node ID
    pub node_id: String,
    /// Run ID
    pub run_id: String,
    /// App ID
    pub app_id: String,
    /// Board ID
    pub board_id: String,
    /// User ID (sub)
    pub user_id: String,
    /// Whether streaming is enabled
    pub stream_state: bool,
    /// Log level (0=Debug, 1=Info, 2=Warn, 3=Error, 4=Fatal)
    pub log_level: u8,
    /// Node name for multi-node packages
    #[serde(default)]
    pub node_name: String,
}

/// Execution result returned from WASM run() function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmExecutionResult {
    /// Output pin values (name -> JSON value)
    pub outputs: serde_json::Map<String, serde_json::Value>,
    /// Error message if execution failed
    #[serde(default)]
    pub error: Option<String>,
    /// Execution pins to activate (names)
    #[serde(default)]
    pub activate_exec: Vec<String>,
    /// Whether execution is still pending (for async operations)
    #[serde(default)]
    pub pending: Option<bool>,
}

impl WasmExecutionResult {
    pub fn success(outputs: serde_json::Map<String, serde_json::Value>) -> Self {
        Self {
            outputs,
            error: None,
            activate_exec: vec!["exec_out".to_string()],
            pending: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            outputs: serde_json::Map::new(),
            error: Some(message.into()),
            activate_exec: vec![],
            pending: None,
        }
    }
}

/// Log entry from WASM module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmLogEntry {
    pub level: u8,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_unpack_ptr_len() {
        let ptr = 0x12345678u32;
        let len = 0x9ABCDEFu32;

        let packed = WasmAbi::pack_ptr_len(ptr, len);
        let (unpacked_ptr, unpacked_len) = WasmAbi::unpack_ptr_len(packed);

        assert_eq!(ptr, unpacked_ptr);
        assert_eq!(len, unpacked_len);
    }

    #[test]
    fn test_error_detection() {
        assert!(WasmAbi::is_error(-1));
        assert!(WasmAbi::is_error(-100));
        assert!(!WasmAbi::is_error(0));
        assert!(!WasmAbi::is_error(WasmAbi::pack_ptr_len(100, 50)));
    }
}
