//! ABI types for WASM nodes

pub use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};

/// Current ABI version
pub const ABI_VERSION: u32 = 1;

/// Permissions a WASM node can request.
/// Declared per-node so the sandbox and UI can enforce/display them precisely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodePermission {
    /// Outbound HTTP requests
    #[serde(rename = "network:http")]
    NetworkHttp,
    /// WebSocket connections
    #[serde(rename = "network:websocket")]
    NetworkWebsocket,
    /// TCP socket access
    #[serde(rename = "network:tcp")]
    NetworkTcp,
    /// UDP socket access
    #[serde(rename = "network:udp")]
    NetworkUdp,
    /// DNS lookups
    #[serde(rename = "network:dns")]
    NetworkDns,
    /// Read from node/user storage
    #[serde(rename = "storage:read")]
    StorageRead,
    /// Write to node/user storage
    #[serde(rename = "storage:write")]
    StorageWrite,
    /// Access flow variables
    #[serde(rename = "variables")]
    Variables,
    /// Access execution cache
    #[serde(rename = "cache")]
    Cache,
    /// Stream responses to the client
    #[serde(rename = "streaming")]
    Streaming,
    /// Access LLM / model providers
    #[serde(rename = "models")]
    Models,
    /// Dynamic UI (Agent-to-UI)
    #[serde(rename = "a2ui")]
    A2ui,
    /// OAuth authentication
    #[serde(rename = "oauth")]
    OAuth,
    /// Call other functions/sub-flows
    #[serde(rename = "functions")]
    Functions,
}

/// The kind of data a pin carries — mirrors the native `VariableType` enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VariableType {
    Execution,
    String,
    Integer,
    Float,
    Boolean,
    Date,
    PathBuf,
    Generic,
    Struct,
    Byte,
}

/// Pin direction — input or output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinType {
    Input,
    Output,
}

/// How the data is contained — scalar or collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueType {
    Normal,
    Array,
    HashMap,
    HashSet,
}

impl Default for ValueType {
    fn default() -> Self {
        Self::Normal
    }
}

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
    pub permissions: Vec<NodePermission>,
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

    /// Add a pre-built pin definition (consuming builder style).
    pub fn add_pin(&mut self, pin: PinDefinition) -> &mut Self {
        self.pins.push(pin);
        self
    }

    /// Add an input pin and return a mutable reference for further configuration.
    /// Mirrors the native catalog's `Node::add_input_pin` builder pattern.
    pub fn add_input_pin(
        &mut self,
        name: &str,
        friendly_name: &str,
        description: &str,
        data_type: VariableType,
    ) -> &mut PinDefinition {
        self.pins.push(PinDefinition::new(
            name,
            friendly_name,
            description,
            PinType::Input,
            data_type,
        ));
        self.pins.last_mut().unwrap()
    }

    /// Add an output pin and return a mutable reference for further configuration.
    /// Mirrors the native catalog's `Node::add_output_pin` builder pattern.
    pub fn add_output_pin(
        &mut self,
        name: &str,
        friendly_name: &str,
        description: &str,
        data_type: VariableType,
    ) -> &mut PinDefinition {
        self.pins.push(PinDefinition::new(
            name,
            friendly_name,
            description,
            PinType::Output,
            data_type,
        ));
        self.pins.last_mut().unwrap()
    }

    pub fn add_icon(&mut self, icon: &str) -> &mut Self {
        self.icon = Some(icon.to_string());
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

    pub fn add_permission(&mut self, permission: NodePermission) -> &mut Self {
        if !self.permissions.contains(&permission) {
            self.permissions.push(permission);
        }
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

/// Pin definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinDefinition {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub pin_type: PinType,
    pub data_type: VariableType,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub value_type: Option<ValueType>,
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
    pub fn new(
        name: &str,
        friendly_name: &str,
        description: &str,
        pin_type: PinType,
        data_type: VariableType,
    ) -> Self {
        Self {
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: description.to_string(),
            pin_type,
            data_type,
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

    pub fn input(
        name: &str,
        friendly_name: &str,
        description: &str,
        data_type: VariableType,
    ) -> Self {
        Self::new(name, friendly_name, description, PinType::Input, data_type)
    }

    pub fn output(
        name: &str,
        friendly_name: &str,
        description: &str,
        data_type: VariableType,
    ) -> Self {
        Self::new(name, friendly_name, description, PinType::Output, data_type)
    }

    // ── Consuming builder methods (for use with `add_pin`) ─────────────

    pub fn with_default(mut self, value: serde_json::Value) -> Self {
        self.default_value = Some(value);
        self
    }

    pub fn with_value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    pub fn with_schema(mut self, schema: &str) -> Self {
        self.schema = Some(schema.to_string());
        self
    }

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

    // ── Mutable reference builder methods (for use with `add_input_pin`/`add_output_pin`) ──

    pub fn set_default_value(&mut self, value: serde_json::Value) -> &mut Self {
        self.default_value = Some(value);
        self
    }

    pub fn set_value_type(&mut self, value_type: ValueType) -> &mut Self {
        self.value_type = Some(value_type);
        self
    }

    pub fn set_schema<T: JsonSchema>(&mut self) -> &mut Self {
        let schema = schemars::schema_for!(T);
        self.schema = Some(serde_json::to_string(&schema).unwrap_or_default());
        self
    }

    pub fn set_schema_raw(&mut self, schema: &str) -> &mut Self {
        self.schema = Some(schema.to_string());
        self
    }

    pub fn set_valid_values(&mut self, values: Vec<String>) -> &mut Self {
        self.valid_values = Some(values);
        self
    }

    pub fn set_range(&mut self, min: f64, max: f64) -> &mut Self {
        self.range = Some((min, max));
        self
    }

    pub fn set_step(&mut self, step: f64) -> &mut Self {
        self.step = Some(step);
        self
    }

    pub fn set_sensitive(&mut self, sensitive: bool) -> &mut Self {
        self.sensitive = Some(sensitive);
        self
    }

    pub fn set_enforce_schema(&mut self, enforce: bool) -> &mut Self {
        self.enforce_schema = Some(enforce);
        self
    }

    pub fn set_enforce_generic_value_type(&mut self, enforce: bool) -> &mut Self {
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
            VariableType::String,
        ));
        node.add_pin(PinDefinition::output(
            "output1",
            "Output 1",
            "First output",
            VariableType::String,
        ));

        assert_eq!(node.pins.len(), 2);
        assert_eq!(node.pins[0].name, "input1");
        assert_eq!(node.pins[1].name, "output1");
    }

    #[test]
    fn test_node_definition_add_input_output_pin() {
        let mut node = NodeDefinition::new("test", "Test", "Test", "Test");
        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        assert_eq!(node.pins.len(), 2);
        assert_eq!(node.pins[0].pin_type, PinType::Input);
        assert_eq!(node.pins[0].data_type, VariableType::Execution);
        assert_eq!(node.pins[1].pin_type, PinType::Output);
    }

    #[test]
    fn test_node_add_input_pin_builder() {
        let mut node = NodeDefinition::new("test", "Test", "Test", "Test");
        node.add_input_pin("config", "Config", "Configuration", VariableType::Struct)
            .set_schema::<TestConfig>()
            .set_enforce_schema(true);

        assert!(node.pins[0].schema.is_some());
        assert_eq!(node.pins[0].enforce_schema, Some(true));
    }

    #[test]
    fn test_pin_definition_input() {
        let pin = PinDefinition::input("name", "Name", "Enter name", VariableType::String);

        assert_eq!(pin.name, "name");
        assert_eq!(pin.pin_type, PinType::Input);
        assert_eq!(pin.data_type, VariableType::String);
    }

    #[test]
    fn test_pin_definition_output() {
        let pin = PinDefinition::output("result", "Result", "The result", VariableType::Integer);

        assert_eq!(pin.name, "result");
        assert_eq!(pin.pin_type, PinType::Output);
        assert_eq!(pin.data_type, VariableType::Integer);
    }

    #[test]
    fn test_pin_definition_with_default() {
        let pin =
            PinDefinition::input("count", "Count", "Number of items", VariableType::Integer)
                .with_default(serde_json::json!(10));

        assert_eq!(pin.default_value, Some(serde_json::json!(10)));
    }

    #[test]
    fn test_pin_definition_with_range() {
        let pin = PinDefinition::input(
            "temperature",
            "Temperature",
            "Temperature value",
            VariableType::Float,
        )
        .with_range(-273.15, 1000.0);

        assert_eq!(pin.range, Some((-273.15, 1000.0)));
    }

    #[test]
    fn test_pin_definition_with_valid_values() {
        let pin = PinDefinition::input("color", "Color", "Color choice", VariableType::String)
            .with_valid_values(vec![
                "red".to_string(),
                "green".to_string(),
                "blue".to_string(),
            ]);

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
    fn test_pin_serialization_matches_runtime() {
        let pin = PinDefinition::input("exec", "Exec", "Trigger", VariableType::Execution);
        let json = serde_json::to_string(&pin).unwrap();
        assert!(json.contains("\"Execution\""));
        assert!(json.contains("\"Input\""));

        let pin = PinDefinition::output("out", "Out", "Output", VariableType::Integer);
        let json = serde_json::to_string(&pin).unwrap();
        assert!(json.contains("\"Integer\""));
        assert!(json.contains("\"Output\""));
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

    #[derive(serde::Serialize, serde::Deserialize, JsonSchema)]
    struct TestConfig {
        threshold: f64,
        label: String,
    }

    #[derive(serde::Serialize, serde::Deserialize, JsonSchema)]
    struct EmailPayload {
        to: String,
        subject: String,
        body: String,
        cc: Option<Vec<String>>,
        priority: u8,
    }

    /// Mirrors the runtime's WasmPinDefinition (all-String fields).
    /// Used to verify SDK enum types deserialize correctly at the ABI boundary.
    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code)]
    struct RuntimePinDefinition {
        name: String,
        friendly_name: String,
        description: String,
        pin_type: String,
        data_type: String,
        default_value: Option<serde_json::Value>,
        value_type: Option<String>,
        schema: Option<String>,
        valid_values: Option<Vec<String>>,
        range: Option<(f64, f64)>,
        step: Option<f64>,
        sensitive: Option<bool>,
        enforce_schema: Option<bool>,
        enforce_generic_value_type: Option<bool>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code)]
    struct RuntimeNodeDefinition {
        name: String,
        friendly_name: String,
        description: String,
        category: String,
        icon: Option<String>,
        pins: Vec<RuntimePinDefinition>,
    }

    fn runtime_map_data_type(wasm_type: &str) -> &'static str {
        match wasm_type.to_lowercase().as_str() {
            "string" => "String",
            "int" | "integer" | "i32" | "i64" | "u32" | "u64" => "Integer",
            "float" | "f32" | "f64" | "number" => "Float",
            "bool" | "boolean" => "Boolean",
            "date" | "datetime" => "Date",
            "path" | "pathbuf" => "PathBuf",
            "byte" | "bytes" | "binary" => "Byte",
            "exec" | "execution" => "Execution",
            "struct" | "object" | "json" => "Struct",
            _ => "Generic",
        }
    }

    #[test]
    fn test_roundtrip_sdk_to_runtime_basic_node() {
        let mut node = NodeDefinition::new("my_node", "My Node", "A test node", "Test");
        node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
        node.add_input_pin("text", "Text", "Input text", VariableType::String)
            .set_default_value(serde_json::json!("hello"));
        node.add_input_pin("count", "Count", "Repeat count", VariableType::Integer)
            .set_default_value(serde_json::json!(3));
        node.add_output_pin("exec_out", "Done", "Continue", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output", VariableType::String);

        let json = serde_json::to_string(&node).unwrap();
        let rt: RuntimeNodeDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(rt.name, "my_node");
        assert_eq!(rt.pins.len(), 5);

        assert_eq!(rt.pins[0].data_type, "Execution");
        assert_eq!(rt.pins[0].pin_type, "Input");
        assert_eq!(runtime_map_data_type(&rt.pins[0].data_type), "Execution");

        assert_eq!(rt.pins[1].data_type, "String");
        assert_eq!(rt.pins[1].default_value, Some(serde_json::json!("hello")));

        assert_eq!(rt.pins[2].data_type, "Integer");
        assert_eq!(rt.pins[2].default_value, Some(serde_json::json!(3)));

        assert_eq!(rt.pins[3].pin_type, "Output");
        assert_eq!(rt.pins[4].data_type, "String");
    }

    #[test]
    fn test_roundtrip_struct_with_schema() {
        let mut node = NodeDefinition::new("email", "Send Email", "Sends email", "IO/Email");
        node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
        node.add_input_pin("payload", "Payload", "Email payload", VariableType::Struct)
            .set_schema::<EmailPayload>()
            .set_enforce_schema(true);
        node.add_output_pin("exec_out", "Done", "Continue", VariableType::Execution);

        let json = serde_json::to_string(&node).unwrap();
        let rt: RuntimeNodeDefinition = serde_json::from_str(&json).unwrap();

        let payload_pin = &rt.pins[1];
        assert_eq!(payload_pin.data_type, "Struct");
        assert_eq!(runtime_map_data_type(&payload_pin.data_type), "Struct");
        assert_eq!(payload_pin.enforce_schema, Some(true));

        let schema_str = payload_pin.schema.as_ref().expect("schema must be set");
        let schema: serde_json::Value = serde_json::from_str(schema_str)
            .expect("schema must be valid JSON");
        let props = schema.get("properties").expect("schema must have properties");
        assert!(props.get("to").is_some(), "schema missing 'to' field");
        assert!(props.get("subject").is_some(), "schema missing 'subject' field");
        assert!(props.get("body").is_some(), "schema missing 'body' field");
        assert!(props.get("cc").is_some(), "schema missing 'cc' field");
        assert!(props.get("priority").is_some(), "schema missing 'priority' field");
    }

    #[test]
    fn test_roundtrip_all_variable_types() {
        let types = [
            (VariableType::Execution, "Execution"),
            (VariableType::String, "String"),
            (VariableType::Integer, "Integer"),
            (VariableType::Float, "Float"),
            (VariableType::Boolean, "Boolean"),
            (VariableType::Date, "Date"),
            (VariableType::PathBuf, "PathBuf"),
            (VariableType::Generic, "Generic"),
            (VariableType::Struct, "Struct"),
            (VariableType::Byte, "Byte"),
        ];

        for (var_type, expected_str) in &types {
            let pin = PinDefinition::input(
                "pin", "Pin", "Test pin", var_type.clone(),
            );
            let json = serde_json::to_string(&pin).unwrap();
            let rt: RuntimePinDefinition = serde_json::from_str(&json).unwrap();
            assert_eq!(&rt.data_type, expected_str,
                "VariableType::{expected_str} did not serialize correctly");
            assert_eq!(runtime_map_data_type(&rt.data_type), *expected_str,
                "runtime map_wasm_data_type failed for {expected_str}");
        }
    }

    #[test]
    fn test_roundtrip_value_types() {
        for (vt, expected) in [
            (ValueType::Normal, "Normal"),
            (ValueType::Array, "Array"),
            (ValueType::HashMap, "HashMap"),
            (ValueType::HashSet, "HashSet"),
        ] {
            let pin = PinDefinition::input("p", "P", "test", VariableType::String)
                .with_value_type(vt);
            let json = serde_json::to_string(&pin).unwrap();
            let rt: RuntimePinDefinition = serde_json::from_str(&json).unwrap();
            assert_eq!(rt.value_type.as_deref(), Some(expected));
        }
    }

    #[test]
    fn test_roundtrip_all_pin_options() {
        let mut node = NodeDefinition::new("opts", "Options", "Pin options", "Test");
        node.add_input_pin("slider", "Slider", "A slider", VariableType::Float)
            .set_range(0.0, 1.0)
            .set_step(0.01)
            .set_default_value(serde_json::json!(0.5));
        node.add_input_pin("secret", "API Key", "Sensitive", VariableType::String)
            .set_sensitive(true);
        node.add_input_pin("choice", "Mode", "Pick one", VariableType::String)
            .set_valid_values(vec!["fast".into(), "slow".into()]);
        node.add_input_pin("items", "Items", "Array of items", VariableType::String)
            .set_value_type(ValueType::Array);

        let json = serde_json::to_string(&node).unwrap();
        let rt: RuntimeNodeDefinition = serde_json::from_str(&json).unwrap();

        let slider = &rt.pins[0];
        assert_eq!(slider.range, Some((0.0, 1.0)));
        assert_eq!(slider.step, Some(0.01));
        assert_eq!(slider.default_value, Some(serde_json::json!(0.5)));

        let secret = &rt.pins[1];
        assert_eq!(secret.sensitive, Some(true));

        let choice = &rt.pins[2];
        assert_eq!(choice.valid_values, Some(vec!["fast".to_string(), "slow".to_string()]));

        let items = &rt.pins[3];
        assert_eq!(items.value_type.as_deref(), Some("Array"));
    }

    #[test]
    fn test_schema_is_valid_json_schema() {
        let mut node = NodeDefinition::new("t", "T", "T", "T");
        node.add_input_pin("cfg", "Config", "Config", VariableType::Struct)
            .set_schema::<TestConfig>();

        let schema_str = node.pins[0].schema.as_ref().unwrap();
        let schema: serde_json::Value = serde_json::from_str(schema_str).unwrap();

        assert!(schema.get("$schema").is_some() || schema.get("type").is_some(),
            "schema should be a valid JSON Schema document");
        let props = schema.get("properties").expect("must have properties");
        assert!(props.get("threshold").is_some());
        assert!(props.get("label").is_some());
    }

    #[test]
    fn test_full_node_json_snapshot() {
        let mut node = NodeDefinition::new(
            "process_email",
            "Process Email",
            "Processes an email payload",
            "IO/Email",
        );
        node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
        node.add_input_pin("email", "Email", "The email", VariableType::Struct)
            .set_schema::<EmailPayload>()
            .set_enforce_schema(true)
            .set_default_value(serde_json::json!({
                "to": "user@example.com",
                "subject": "Hello",
                "body": "World",
                "cc": null,
                "priority": 1
            }));
        node.add_output_pin("exec_out", "Done", "Continue", VariableType::Execution);
        node.add_output_pin("status", "Status", "Result code", VariableType::Integer);

        let json = serde_json::to_string_pretty(&node).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let pins = parsed["pins"].as_array().unwrap();
        assert_eq!(pins.len(), 4);

        let email_pin = &pins[1];
        assert_eq!(email_pin["data_type"], "Struct");
        assert_eq!(email_pin["enforce_schema"], true);
        assert!(email_pin["schema"].is_string());

        let schema: serde_json::Value =
            serde_json::from_str(email_pin["schema"].as_str().unwrap()).unwrap();
        assert!(schema["properties"]["to"].is_object());
        assert!(schema["properties"]["subject"].is_object());
    }
}
