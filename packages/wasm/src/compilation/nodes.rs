use crate::error::{WasmError, WasmResult};
use flow_like_wasm_schema::node_definition::{WasmNodeDefinition, WasmNodeScores};
use flow_like_wasm_schema::package_node::PackageNodeMetadata;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Package metadata returned to the API. Pins remain wire data until an execution
/// host chooses to convert them into runtime objects.
pub type ExtractedNode = PackageNodeMetadata<Value, WasmNodeScores, Value, String>;

pub(super) fn parse(json: &str) -> WasmResult<Vec<ExtractedNode>> {
    let definitions = serde_json::from_str::<Vec<WasmNodeDefinition>>(json)
        .or_else(|_| serde_json::from_str::<WasmNodeDefinition>(json).map(|node| vec![node]))
        .map_err(|error| WasmError::invalid_node_definition(error.to_string()))?;
    definitions.into_iter().map(convert).collect()
}

fn convert(definition: WasmNodeDefinition) -> WasmResult<ExtractedNode> {
    const PERMISSIONS: &[&str] = &[
        "network:http",
        "network:websocket",
        "network:tcp",
        "network:udp",
        "network:dns",
        "storage:read",
        "storage:write",
        "database:read",
        "database:write",
        "variables",
        "cache",
        "streaming",
        "models",
        "a2ui",
        "oauth",
        "functions",
    ];
    for permission in &definition.permissions {
        if !PERMISSIONS.contains(&permission.as_str()) {
            return Err(WasmError::invalid_node_definition(format!(
                "Unknown node permission: {permission}"
            )));
        }
    }
    let mut pins = HashMap::new();
    for (index, pin) in definition.pins.iter().enumerate() {
        let data_type = match pin.data_type.to_lowercase().as_str() {
            "string" => "String",
            "int" | "integer" | "i32" | "i64" | "u32" | "u64" => "Integer",
            "float" | "f32" | "f64" | "number" => "Float",
            "bool" | "boolean" => "Boolean",
            "date" | "datetime" => "Date",
            "path" | "pathbuf" => "PathBuf",
            "byte" | "bytes" | "binary" => "Byte",
            "exec" | "execution" => "Execution",
            "struct" | "object" | "json" => "Struct",
            "geometry" => "Geometry",
            _ => "Generic",
        };
        let value_type = match pin
            .value_type
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "array" => "Array",
            "hashmap" => "HashMap",
            "hashset" => "HashSet",
            _ => "Normal",
        };
        let has_options = pin.valid_values.is_some()
            || pin.range.is_some()
            || pin.step.is_some()
            || pin.sensitive.is_some()
            || pin.enforce_schema.is_some()
            || pin.enforce_generic_value_type.is_some();
        let options = has_options.then(|| {
            json!({
                "valid_values": pin.valid_values, "range": pin.range, "step": pin.step,
                "sensitive": pin.sensitive, "enforce_schema": pin.enforce_schema,
                "enforce_generic_value_type": pin.enforce_generic_value_type, "optional": null,
            })
        });
        let default_value = pin
            .default_value
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| WasmError::invalid_node_definition(error.to_string()))?;
        pins.insert(pin.name.clone(), json!({
            "id": cuid2::create_id(), "name": pin.name, "friendly_name": pin.friendly_name,
            "description": pin.description,
            "pin_type": if pin.pin_type.eq_ignore_ascii_case("output") { "Output" } else { "Input" },
            "data_type": data_type, "schema": pin.schema, "value_type": value_type,
            "depends_on": [], "connected_to": [], "default_value": default_value,
            "index": index as u16, "options": options,
        }));
    }
    Ok(ExtractedNode {
        id: definition.name.clone(),
        name: definition.name,
        friendly_name: Some(definition.friendly_name),
        description: definition.description,
        category: definition.category,
        icon: definition.icon,
        scores: definition.scores,
        pins,
        start: None,
        long_running: definition.long_running,
        docs: definition.docs,
        event_callback: None,
        fn_refs: None,
        oauth_providers: vec![],
        required_oauth_scopes: None,
        only_offline: false,
        version: None,
        permissions: definition.permissions,
        metadata: HashMap::new(),
    })
}
