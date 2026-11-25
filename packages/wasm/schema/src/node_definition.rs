//! Guest node definitions, independent of the execution runtime.

use serde::{Deserialize, Serialize};

/// Node definition as returned by WASM module's get_node()
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "Permission: Deserialize<'de>"))]
pub struct WasmNodeDefinition<Permission = String> {
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
    pub permissions: Vec<Permission>,
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
