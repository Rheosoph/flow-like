use crate::context::Context;
use crate::types::{ExecutionInput, LogLevel, NodeDefinition, PinType, VariableType};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MockHost {
    pub logs: Vec<(u8, String)>,
    pub streams: Vec<(String, String)>,
    pub variables: HashMap<String, Value>,
    pub inputs: HashMap<String, Value>,
    pub outputs: HashMap<String, Value>,
    pub exec_pins: Vec<String>,
    pub time: i64,
    pub random_value: u64,
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHost {
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
            streams: Vec::new(),
            variables: HashMap::new(),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            exec_pins: Vec::new(),
            time: 0,
            random_value: 0,
        }
    }

    pub fn with_input(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.inputs.insert(name.into(), value.into());
        self
    }

    pub fn with_variable(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.variables.insert(name.into(), value.into());
        self
    }

    pub fn has_log(&self, level: u8, substring: &str) -> bool {
        self.logs
            .iter()
            .any(|(l, msg)| *l == level && msg.contains(substring))
    }

    pub fn has_debug(&self, substring: &str) -> bool {
        self.has_log(LogLevel::Debug as u8, substring)
    }

    pub fn has_info(&self, substring: &str) -> bool {
        self.has_log(LogLevel::Info as u8, substring)
    }

    pub fn has_warn(&self, substring: &str) -> bool {
        self.has_log(LogLevel::Warn as u8, substring)
    }

    pub fn has_error(&self, substring: &str) -> bool {
        self.has_log(LogLevel::Error as u8, substring)
    }

    pub fn has_stream(&self, event_type: &str, substring: &str) -> bool {
        self.streams
            .iter()
            .any(|(t, d)| t == event_type && d.contains(substring))
    }
}

pub struct MockContext {
    pub context: Context,
    pub host: MockHost,
}

impl MockContext {
    pub fn new_with_inputs(inputs: HashMap<String, Value>) -> Self {
        let input = ExecutionInput {
            inputs: {
                let mut map = serde_json::Map::new();
                for (k, v) in &inputs {
                    map.insert(k.clone(), v.clone());
                }
                map
            },
            node_id: "mock-node".to_string(),
            run_id: "mock-run".to_string(),
            app_id: "mock-app".to_string(),
            board_id: "mock-board".to_string(),
            user_id: "mock-user".to_string(),
            stream_state: true,
            log_level: 0,
            node_name: String::new(),
        };

        Self {
            context: Context::from_input(input),
            host: MockHost {
                inputs,
                ..MockHost::new()
            },
        }
    }

    pub fn from_definition(def: &NodeDefinition, overrides: HashMap<String, Value>) -> Self {
        let mut inputs = HashMap::new();

        for pin in &def.pins {
            if pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution {
                if let Some(default) = &pin.default_value {
                    inputs.insert(pin.name.clone(), default.clone());
                }
            }
        }

        for (k, v) in overrides {
            inputs.insert(k, v);
        }

        Self::new_with_inputs(inputs)
    }
}

impl std::ops::Deref for MockContext {
    type Target = Context;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

impl std::ops::DerefMut for MockContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PinDefinition, VariableType};
    use serde_json::json;

    #[test]
    fn test_mock_host_builder() {
        let host = MockHost::new()
            .with_input("name", json!("Alice"))
            .with_variable("count", json!(42));

        assert_eq!(host.inputs.get("name"), Some(&json!("Alice")));
        assert_eq!(host.variables.get("count"), Some(&json!(42)));
    }

    #[test]
    fn test_mock_context_new_with_inputs() {
        let mut inputs = HashMap::new();
        inputs.insert("text".to_string(), json!("hello"));
        inputs.insert("count".to_string(), json!(5));

        let ctx = MockContext::new_with_inputs(inputs);

        assert_eq!(ctx.get_string("text"), Some("hello".to_string()));
        assert_eq!(ctx.get_i64("count"), Some(5));
        assert_eq!(ctx.node_id(), "mock-node");
    }

    #[test]
    fn test_mock_context_from_definition() {
        let mut def = NodeDefinition::new("test", "Test", "A test node", "Test");
        def.add_pin(
            PinDefinition::input("value", "Value", "Input value", VariableType::Integer)
                .with_default(json!(10)),
        );
        def.add_pin(
            PinDefinition::input("label", "Label", "A label", VariableType::String)
                .with_default(json!("default")),
        );
        def.add_pin(PinDefinition::output(
            "result",
            "Result",
            "Output",
            VariableType::Integer,
        ));

        let mut overrides = HashMap::new();
        overrides.insert("value".to_string(), json!(99));

        let ctx = MockContext::from_definition(&def, overrides);

        assert_eq!(ctx.get_i64("value"), Some(99));
        assert_eq!(ctx.get_string("label"), Some("default".to_string()));
    }

    #[test]
    fn test_mock_context_set_output_and_finish() {
        let ctx = MockContext::new_with_inputs(HashMap::new());
        let mut inner = ctx.context;
        inner.set_output("result", json!(42));
        let result = inner.success();

        assert_eq!(result.outputs.get("result"), Some(&json!(42)));
        assert!(result.activate_exec.contains(&"exec_out".to_string()));
    }

    #[test]
    fn test_mock_host_log_checks() {
        let mut host = MockHost::new();
        host.logs
            .push((LogLevel::Info as u8, "started processing".to_string()));
        host.logs
            .push((LogLevel::Error as u8, "something failed".to_string()));

        assert!(host.has_info("started"));
        assert!(host.has_error("failed"));
        assert!(!host.has_warn("anything"));
    }

    #[test]
    fn test_mock_host_stream_checks() {
        let mut host = MockHost::new();
        host.streams
            .push(("text".to_string(), "hello world".to_string()));
        host.streams
            .push(("progress".to_string(), r#"{"progress":0.5}"#.to_string()));

        assert!(host.has_stream("text", "hello"));
        assert!(host.has_stream("progress", "0.5"));
        assert!(!host.has_stream("json", "anything"));
    }
}
