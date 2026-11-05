//! Calling a package node as a bot skill tool, outside a board: its input pins become the
//! tool's arguments by name, and its schema is what `skill_load` shows the bot. The run itself
//! goes through [`crate::package_runtime::PackageRuntime::call`] with a `HostState` the host
//! builds (workspace storage, the bot's model policy, http limited to `allowed_hosts`).

use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like_wasm_schema::manifest::PackageNodeEntry;
use serde_json::{Map, Value, json};

use crate::abi::WasmExecutionInput;

/// What a bot sees of one package node.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

fn data_pins(entry: &PackageNodeEntry) -> Vec<&Pin> {
    let mut pins: Vec<&Pin> = entry
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution)
        .collect();
    pins.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.name.cmp(&b.name)));
    pins
}

fn json_type(pin: &Pin) -> Value {
    let mut schema = match pin.data_type {
        VariableType::String | VariableType::Date | VariableType::PathBuf => {
            json!({ "type": "string" })
        }
        VariableType::Integer => json!({ "type": "integer" }),
        VariableType::Float => json!({ "type": "number" }),
        VariableType::Boolean => json!({ "type": "boolean" }),
        VariableType::Byte => json!({ "type": "array", "items": { "type": "integer" } }),
        VariableType::Struct | VariableType::Geometry => pin
            .schema
            .as_deref()
            .and_then(|schema| serde_json::from_str::<Value>(schema).ok())
            .unwrap_or_else(|| json!({ "type": "object" })),
        VariableType::Generic | VariableType::Execution => json!({}),
    };
    if let (Some(object), false) = (schema.as_object_mut(), pin.description.is_empty()) {
        object.insert("description".into(), Value::String(pin.description.clone()));
    }
    schema
}

fn has_default(pin: &Pin) -> bool {
    pin.default_value.as_ref().is_some_and(|bytes| !bytes.is_empty())
}

/// The node as a tool: its data input pins as an object schema, pins without a default required.
pub fn tool_info(entry: &PackageNodeEntry) -> SkillToolInfo {
    let pins = data_pins(entry);
    let properties: Map<String, Value> = pins
        .iter()
        .map(|pin| (pin.name.clone(), json_type(pin)))
        .collect();
    let required: Vec<&str> = pins
        .iter()
        .filter(|pin| !has_default(pin))
        .map(|pin| pin.name.as_str())
        .collect();
    SkillToolInfo {
        name: entry.name.clone(),
        description: entry.description.clone(),
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

/// The bot's arguments as the node's input pin values. A name that is no input pin and a
/// missing pin without a default are refused with the names that are valid.
pub fn inputs_for_arguments(
    entry: &PackageNodeEntry,
    arguments: &Value,
) -> Result<Map<String, Value>, String> {
    let given = match arguments {
        Value::Null => &Map::new(),
        Value::Object(object) => object,
        _ => return Err("The arguments are an object of pin values".to_string()),
    };
    let pins = data_pins(entry);
    let names: Vec<&str> = pins.iter().map(|pin| pin.name.as_str()).collect();
    if let Some(unknown) = given.keys().find(|key| !names.contains(&key.as_str())) {
        return Err(format!(
            "'{unknown}' is no input of {}; its inputs are: {}",
            entry.name,
            names.join(", ")
        ));
    }
    let missing: Vec<&str> = pins
        .iter()
        .filter(|pin| !has_default(pin) && !given.contains_key(&pin.name))
        .map(|pin| pin.name.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{} needs these inputs: {}",
            entry.name,
            missing.join(", ")
        ));
    }
    Ok(given.clone())
}

/// The run input of one skill call. The run is the bot's turn and the app its workspace.
pub fn run_input(
    entry: &PackageNodeEntry,
    inputs: Map<String, Value>,
    workspace: &str,
    turn_id: &str,
    user_id: &str,
) -> WasmExecutionInput {
    WasmExecutionInput {
        inputs,
        node_id: format!("skill-{}", entry.name),
        run_id: turn_id.to_string(),
        app_id: workspace.to_string(),
        board_id: String::new(),
        user_id: user_id.to_string(),
        stream_state: false,
        log_level: 1,
        node_name: entry.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(name: &str, data_type: VariableType, default: Option<&[u8]>, index: u16) -> Pin {
        let mut pin = Pin {
            id: name.into(),
            name: name.into(),
            friendly_name: name.into(),
            description: format!("the {name}"),
            pin_type: PinType::Input,
            data_type,
            schema: None,
            value_type: flow_like::flow::pin::ValueType::Normal,
            depends_on: Default::default(),
            connected_to: Default::default(),
            default_value: default.map(<[u8]>::to_vec),
            index,
            options: None,
            value: None,
        };
        pin.pin_type = PinType::Input;
        pin
    }

    fn entry() -> PackageNodeEntry {
        let mut entry: PackageNodeEntry = serde_json::from_value(json!({
            "id": "extract", "name": "extract", "description": "Extract text", "category": "Docs"
        }))
        .unwrap();
        for pin in [
            pin("exec", VariableType::Execution, None, 0),
            pin("path", VariableType::String, None, 1),
            pin("page", VariableType::Integer, Some(b"1"), 2),
        ] {
            entry.pins.insert(pin.name.clone(), pin);
        }
        entry
    }

    #[test]
    fn the_tool_schema_lists_data_inputs_and_requires_pins_without_a_default() {
        let info = tool_info(&entry());
        let properties = info.input_schema["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 2, "the execution pin is no argument");
        assert_eq!(properties["page"]["type"], "integer");
        assert_eq!(info.input_schema["required"], json!(["path"]));
    }

    #[test]
    fn arguments_map_to_pins_by_name() {
        let inputs = inputs_for_arguments(&entry(), &json!({ "path": "a.pdf" })).unwrap();
        assert_eq!(inputs["path"], "a.pdf");
        let error = inputs_for_arguments(&entry(), &json!({ "path": "a", "nope": 1 })).unwrap_err();
        assert!(error.contains("'nope' is no input") && error.contains("path, page"), "{error}");
        let error = inputs_for_arguments(&entry(), &json!({})).unwrap_err();
        assert!(error.contains("needs these inputs: path"), "{error}");
        assert!(inputs_for_arguments(&entry(), &json!([1])).is_err());
    }

    #[test]
    fn the_run_belongs_to_the_turn_and_the_workspace_and_no_board() {
        let input = run_input(&entry(), Map::new(), "ws-1", "turn-1", "owner");
        assert_eq!(input.app_id, "ws-1");
        assert_eq!(input.run_id, "turn-1");
        assert!(input.board_id.is_empty());
        assert_eq!(input.node_name, "extract");
    }
}
