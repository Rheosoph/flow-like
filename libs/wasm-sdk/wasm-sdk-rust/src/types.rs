//! ABI types for WASM nodes

pub use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};

/// Current ABI version
pub const ABI_VERSION: u32 = 1;

/// Node definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub pins: Vec<PinDefinition>,
    #[serde(default)]
    pub scores: Option<NodeScores>,
    #[serde(default)]
    pub long_running: Option<bool>,
    #[serde(default)]
    pub docs: Option<String>,
    #[serde(default)]
    pub abi_version: Option<u32>,
    /// Per-node permissions. Empty means no additional permissions needed.
    #[serde(default)]
    pub permissions: Vec<String>,
}

impl NodeDefinition {
    pub fn new(name: &str, friendly_name: &str, description: &str, category: &str) -> Self {
        Self {
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: description.to_string(),
            category: category.to_string(),
            icon: None,
            pins: Vec::new(),
            scores: None,
            long_running: None,
            docs: None,
            abi_version: Some(ABI_VERSION),
            permissions: Vec::new(),
        }
    }

    pub fn add_pin(&mut self, pin: PinDefinition) -> &mut Self {
        self.pins.push(pin);
        self
    }

    pub fn set_scores(&mut self, scores: NodeScores) -> &mut Self {
        self.scores = Some(scores);
        self
    }

    pub fn set_long_running(&mut self, long_running: bool) -> &mut Self {
        self.long_running = Some(long_running);
        self
    }

    pub fn add_permission(&mut self, permission: &str) -> &mut Self {
        self.permissions.push(permission.to_string());
        self
    }
}

/// Multiple node definitions for a package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageNodes {
    #[serde(flatten)]
    pub nodes: Vec<NodeDefinition>,
}

impl Default for PackageNodes {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageNodes {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn add_node(&mut self, node: NodeDefinition) -> &mut Self {
        self.nodes.push(node);
        self
    }

    /// Serialize nodes into JSON string for Component Model return
    pub fn to_wasm(&self) -> String {
        serde_json::to_string(&self.nodes).unwrap_or_default()
    }
}

/// Data type constants — the kind of data a pin carries.
///
/// These match the core `VariableType` enum. Use them as the `data_type`
/// argument when creating pins.
#[allow(non_snake_case)]
pub mod DataType {
    pub const EXEC: &str = "Exec";
    pub const STRING: &str = "String";
    pub const I64: &str = "I64";
    pub const F64: &str = "F64";
    pub const BOOL: &str = "Bool";
    pub const GENERIC: &str = "Generic";
    pub const BYTES: &str = "Bytes";
    pub const DATE: &str = "Date";
    pub const PATH_BUF: &str = "PathBuf";
    pub const STRUCT: &str = "Struct";
}

/// Value type constants — how the data is contained (scalar vs collection).
///
/// These match the core `ValueType` enum. Use them with
/// [`PinDefinition::with_value_type`].
#[allow(non_snake_case)]
pub mod ValueType {
    pub const NORMAL: &str = "Normal";
    pub const ARRAY: &str = "Array";
    pub const HASH_MAP: &str = "HashMap";
    pub const HASH_SET: &str = "HashSet";
}

/// Pin definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinDefinition {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub pin_type: String,
    pub data_type: String,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub value_type: Option<String>,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub valid_values: Option<Vec<String>>,
    #[serde(default)]
    pub range: Option<(f64, f64)>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub sensitive: Option<bool>,
    #[serde(default)]
    pub enforce_schema: Option<bool>,
    #[serde(default)]
    pub enforce_generic_value_type: Option<bool>,
}

impl PinDefinition {
    pub fn input(name: &str, friendly_name: &str, description: &str, data_type: &str) -> Self {
        Self {
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: description.to_string(),
            pin_type: "Input".to_string(),
            data_type: data_type.to_string(),
            default_value: None,
            value_type: None,
            schema: None,
            valid_values: None,
            range: None,
            step: None,
            sensitive: None,
            enforce_schema: None,
            enforce_generic_value_type: None,
        }
    }

    pub fn output(name: &str, friendly_name: &str, description: &str, data_type: &str) -> Self {
        Self {
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: description.to_string(),
            pin_type: "Output".to_string(),
            data_type: data_type.to_string(),
            default_value: None,
            value_type: None,
            schema: None,
            valid_values: None,
            range: None,
            step: None,
            sensitive: None,
            enforce_schema: None,
            enforce_generic_value_type: None,
        }
    }

    pub fn with_default(mut self, value: serde_json::Value) -> Self {
        self.default_value = Some(value);
        self
    }

    pub fn with_value_type(mut self, value_type: &str) -> Self {
        self.value_type = Some(value_type.to_string());
        self
    }

    pub fn with_schema(mut self, schema: &str) -> Self {
        self.schema = Some(schema.to_string());
        self
    }

    /// Derive a JSON Schema from a type that implements [`JsonSchema`] and
    /// attach it to this pin in one step.
    ///
    /// ```rust,ignore
    /// use flow_like_wasm_sdk::{JsonSchema, PinDefinition, DataType};
    ///
    /// #[derive(serde::Serialize, serde::Deserialize, JsonSchema)]
    /// struct Config { threshold: f64, label: String }
    ///
    /// let pin = PinDefinition::input("config", "Config", "Node config", DataType::STRUCT)
    ///     .with_schema_type::<Config>();
    /// ```
    pub fn with_schema_type<T: JsonSchema>(self) -> Self {
        let schema = schemars::schema_for!(T);
        let schema_str = serde_json::to_string(&schema).unwrap_or_default();
        self.with_schema(&schema_str)
    }

    pub fn with_valid_values(mut self, values: Vec<String>) -> Self {
        self.valid_values = Some(values);
        self
    }

    pub fn with_range(mut self, min: f64, max: f64) -> Self {
        self.range = Some((min, max));
        self
    }

    pub fn with_step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    pub fn with_sensitive(mut self, sensitive: bool) -> Self {
        self.sensitive = Some(sensitive);
        self
    }

    pub fn with_enforce_schema(mut self, enforce: bool) -> Self {
        self.enforce_schema = Some(enforce);
        self
    }

    pub fn with_enforce_generic_value_type(mut self, enforce: bool) -> Self {
        self.enforce_generic_value_type = Some(enforce);
        self
    }
}

/// Node quality scores
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeScores {
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

/// Execution input from the host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionInput {
    pub inputs: serde_json::Map<String, serde_json::Value>,
    pub node_id: String,
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
    pub user_id: String,
    pub stream_state: bool,
    pub log_level: u8,
    /// Node name for multi-node packages (optional)
    #[serde(default)]
    pub node_name: String,
}

/// Execution result to return to host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub outputs: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub activate_exec: Vec<String>,
    #[serde(default)]
    pub pending: Option<bool>,
}

impl ExecutionResult {
    pub fn success() -> Self {
        Self {
            outputs: serde_json::Map::new(),
            error: None,
            activate_exec: Vec::new(),
            pending: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            outputs: serde_json::Map::new(),
            error: Some(message.into()),
            activate_exec: Vec::new(),
            pending: None,
        }
    }

    pub fn set_output(&mut self, name: &str, value: serde_json::Value) -> &mut Self {
        self.outputs.insert(name.to_string(), value);
        self
    }

    pub fn activate_exec(&mut self, pin_name: &str) -> &mut Self {
        self.activate_exec.push(pin_name.to_string());
        self
    }

    pub fn set_pending(&mut self, pending: bool) -> &mut Self {
        self.pending = Some(pending);
        self
    }

    /// Serialize result into JSON string for Component Model return
    pub fn to_wasm(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Log levels (matches core `flow_like::flow::execution::LogLevel`)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Fatal = 4,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_definition_new() {
        let node = NodeDefinition::new("test", "Test Node", "A test node", "Test/Category");

        assert_eq!(node.name, "test");
        assert_eq!(node.friendly_name, "Test Node");
        assert_eq!(node.description, "A test node");
        assert_eq!(node.category, "Test/Category");
        assert_eq!(node.abi_version, Some(ABI_VERSION));
    }

    #[test]
    fn test_node_definition_add_pin() {
        let mut node = NodeDefinition::new("test", "Test", "Test", "Test");
        node.add_pin(PinDefinition::input(
            "input1",
            "Input 1",
            "First input",
            "String",
        ));
        node.add_pin(PinDefinition::output(
            "output1",
            "Output 1",
            "First output",
            "String",
        ));

        assert_eq!(node.pins.len(), 2);
        assert_eq!(node.pins[0].name, "input1");
        assert_eq!(node.pins[1].name, "output1");
    }

    #[test]
    fn test_pin_definition_input() {
        let pin = PinDefinition::input("name", "Name", "Enter name", "String");

        assert_eq!(pin.name, "name");
        assert_eq!(pin.pin_type, "Input");
        assert_eq!(pin.data_type, "String");
    }

    #[test]
    fn test_pin_definition_output() {
        let pin = PinDefinition::output("result", "Result", "The result", "I64");

        assert_eq!(pin.name, "result");
        assert_eq!(pin.pin_type, "Output");
        assert_eq!(pin.data_type, "I64");
    }

    #[test]
    fn test_pin_definition_with_default() {
        let pin = PinDefinition::input("count", "Count", "Number of items", "I64")
            .with_default(serde_json::json!(10));

        assert_eq!(pin.default_value, Some(serde_json::json!(10)));
    }

    #[test]
    fn test_pin_definition_with_range() {
        let pin = PinDefinition::input("temperature", "Temperature", "Temperature value", "F64")
            .with_range(-273.15, 1000.0);

        assert_eq!(pin.range, Some((-273.15, 1000.0)));
    }

    #[test]
    fn test_pin_definition_with_valid_values() {
        let pin =
            PinDefinition::input("color", "Color", "Color choice", "String").with_valid_values(
                vec!["red".to_string(), "green".to_string(), "blue".to_string()],
            );

        assert_eq!(
            pin.valid_values,
            Some(vec![
                "red".to_string(),
                "green".to_string(),
                "blue".to_string()
            ])
        );
    }

    #[test]
    fn test_execution_result_success() {
        let result = ExecutionResult::success();

        assert!(result.error.is_none());
        assert!(result.outputs.is_empty());
        assert!(result.activate_exec.is_empty());
    }

    #[test]
    fn test_execution_result_error() {
        let result = ExecutionResult::error("Something failed");

        assert_eq!(result.error, Some("Something failed".to_string()));
    }

    #[test]
    fn test_execution_result_set_output() {
        let mut result = ExecutionResult::success();
        result.set_output("value", serde_json::json!(42));

        assert_eq!(result.outputs.get("value"), Some(&serde_json::json!(42)));
    }

    #[test]
    fn test_execution_result_activate_exec() {
        let mut result = ExecutionResult::success();
        result.activate_exec("branch_a");
        result.activate_exec("branch_b");

        assert!(result.activate_exec.contains(&"branch_a".to_string()));
        assert!(result.activate_exec.contains(&"branch_b".to_string()));
    }

    #[test]
    fn test_execution_result_set_pending() {
        let mut result = ExecutionResult::success();
        result.set_pending(true);

        assert_eq!(result.pending, Some(true));
    }

    #[test]
    fn test_node_scores_default() {
        let scores = NodeScores::default();

        assert_eq!(scores.privacy, 0);
        assert_eq!(scores.security, 0);
        assert_eq!(scores.performance, 0);
    }

    #[test]
    fn test_package_nodes() {
        let mut package = PackageNodes::new();
        package.add_node(NodeDefinition::new("node1", "Node 1", "First node", "Test"));
        package.add_node(NodeDefinition::new(
            "node2",
            "Node 2",
            "Second node",
            "Test",
        ));

        assert_eq!(package.nodes.len(), 2);
    }

    #[test]
    fn test_node_definition_serialization() {
        let node = NodeDefinition::new("test", "Test", "A test", "Test/Cat");
        let json = serde_json::to_string(&node).unwrap();

        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"friendly_name\":\"Test\""));
    }

    #[test]
    fn test_execution_result_serialization() {
        let mut result = ExecutionResult::success();
        result.set_output("count", serde_json::json!(100));
        result.activate_exec("exec_out");

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ExecutionResult = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.outputs.get("count"), Some(&serde_json::json!(100)));
        assert!(parsed.activate_exec.contains(&"exec_out".to_string()));
    }
}
