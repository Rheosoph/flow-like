use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::{collections::HashMap, sync::Arc};

/// Unique identifier prefix for make struct pins to enable special connection rules
pub const MAKE_STRUCT_PIN_PREFIX: &str = "__make_struct_field__";

#[crate::register_node]
#[derive(Default)]
pub struct MakeStructFromSchemaNode {}

impl MakeStructFromSchemaNode {
    pub fn new() -> Self {
        MakeStructFromSchemaNode {}
    }
}

/// Resolve a $ref reference to its definition in the schema
fn resolve_ref<'a>(ref_path: &str, root_schema: &'a Value) -> Option<&'a Value> {
    // $ref format: "#/definitions/TypeName" or "#/$defs/TypeName"
    let path = ref_path.strip_prefix("#/")?;
    let parts: Vec<&str> = path.split('/').collect();

    let mut current = root_schema;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

/// Resolve a schema that might contain $ref, anyOf, or be direct
fn resolve_schema<'a>(schema: &'a Value, root_schema: &'a Value) -> &'a Value {
    // Handle $ref
    if let Some(ref_path) = schema.get("$ref").and_then(|r| r.as_str())
        && let Some(resolved) = resolve_ref(ref_path, root_schema)
    {
        return resolved;
    }

    // Handle anyOf (often used for nullable types)
    if let Some(any_of) = schema.get("anyOf").and_then(|a| a.as_array()) {
        for variant in any_of {
            // Skip null types
            if variant.get("type").and_then(|t| t.as_str()) == Some("null") {
                continue;
            }
            // Recursively resolve the non-null variant
            return resolve_schema(variant, root_schema);
        }
    }

    schema
}

/// Get the variable type from a resolved schema
fn get_schema_type(schema: &Value, root_schema: &Value) -> (VariableType, ValueType) {
    let resolved = resolve_schema(schema, root_schema);

    // Check for array type
    if let Some(type_val) = resolved.get("type") {
        if let Some(type_str) = type_val.as_str() {
            return match type_str {
                "boolean" => (VariableType::Boolean, ValueType::Normal),
                "integer" => (VariableType::Integer, ValueType::Normal),
                "number" => (VariableType::Float, ValueType::Normal),
                "string" => (VariableType::String, ValueType::Normal),
                "array" => {
                    // For arrays, check what the items type is
                    if let Some(items) = resolved.get("items") {
                        let item_resolved = resolve_schema(items, root_schema);
                        let item_type = item_resolved.get("type").and_then(|t| t.as_str());
                        match item_type {
                            Some("boolean") => (VariableType::Boolean, ValueType::Array),
                            Some("integer") => (VariableType::Integer, ValueType::Array),
                            Some("number") => (VariableType::Float, ValueType::Array),
                            Some("string") => (VariableType::String, ValueType::Array),
                            Some("object") | None => (VariableType::Struct, ValueType::Array),
                            _ => (VariableType::Generic, ValueType::Array),
                        }
                    } else {
                        (VariableType::Generic, ValueType::Array)
                    }
                }
                "object" => (VariableType::Struct, ValueType::Normal),
                _ => (VariableType::Generic, ValueType::Normal),
            };
        }
        // Handle array of types (e.g., ["string", "null"])
        if let Some(types) = type_val.as_array() {
            for t in types {
                if let Some(ts) = t.as_str()
                    && ts != "null"
                {
                    return match ts {
                        "boolean" => (VariableType::Boolean, ValueType::Normal),
                        "integer" => (VariableType::Integer, ValueType::Normal),
                        "number" => (VariableType::Float, ValueType::Normal),
                        "string" => (VariableType::String, ValueType::Normal),
                        "array" => (VariableType::Generic, ValueType::Array),
                        "object" => (VariableType::Struct, ValueType::Normal),
                        _ => (VariableType::Generic, ValueType::Normal),
                    };
                }
            }
        }
    }

    // If it has properties or $ref to an object, treat as struct
    if resolved.get("properties").is_some() {
        return (VariableType::Struct, ValueType::Normal);
    }

    (VariableType::Generic, ValueType::Normal)
}

/// Build a standalone schema for a property, inlining any $ref definitions
fn build_standalone_schema(schema: &Value, root_schema: &Value) -> Value {
    let resolved = resolve_schema(schema, root_schema);

    // For objects, build a complete schema with properties
    if resolved.get("type").and_then(|t| t.as_str()) == Some("object")
        || resolved.get("properties").is_some()
    {
        let mut new_schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object"
        });

        if let Some(props) = resolved.get("properties") {
            new_schema["properties"] = props.clone();
        }
        if let Some(required) = resolved.get("required") {
            new_schema["required"] = required.clone();
        }
        if let Some(defs) = root_schema.get("definitions") {
            new_schema["definitions"] = defs.clone();
        } else if let Some(defs) = root_schema.get("$defs") {
            new_schema["$defs"] = defs.clone();
        }

        return new_schema;
    }

    // For arrays, build schema with items
    if resolved.get("type").and_then(|t| t.as_str()) == Some("array") {
        let mut new_schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "array"
        });

        if let Some(items) = resolved.get("items") {
            new_schema["items"] = items.clone();
        }
        if let Some(defs) = root_schema.get("definitions") {
            new_schema["definitions"] = defs.clone();
        } else if let Some(defs) = root_schema.get("$defs") {
            new_schema["$defs"] = defs.clone();
        }

        return new_schema;
    }

    // For primitives, return as-is
    resolved.clone()
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn get_default_value_for_type(var_type: &VariableType, value_type: &ValueType) -> Option<Value> {
    if *value_type == ValueType::Array {
        return Some(json!([]));
    }
    match var_type {
        VariableType::Boolean => Some(json!(false)),
        VariableType::Integer => Some(json!(0)),
        VariableType::Float => Some(json!(0.0)),
        VariableType::String => Some(json!("")),
        VariableType::Struct => Some(json!({})),
        _ => None,
    }
}

#[async_trait]
impl NodeLogic for MakeStructFromSchemaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_make_from_schema",
            "Make Struct (Schema)",
            "Creates a struct from individual fields based on a connected schema",
            "Structs",
        );
        node.add_icon("/flow/icons/struct.svg");

        // Output struct pin - will get schema from connected input
        node.add_output_pin(
            "struct_out",
            "Struct",
            "The constructed struct",
            VariableType::Struct,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut result: HashMap<String, Value> = HashMap::new();

        // Get all input pins and build the struct
        for (_id, pin) in context.node.pins.iter() {
            // Skip output pins and execution pins
            if pin.pin_type != PinType::Input || pin.data_type == VariableType::Execution {
                continue;
            }

            let pin_name = &pin.name;

            // Extract field name from the prefixed pin name
            let field_name = pin_name
                .strip_prefix(MAKE_STRUCT_PIN_PREFIX)
                .unwrap_or(pin_name);

            let value: Value = context.evaluate_pin_ref(pin.clone()).await?;
            result.insert(field_name.to_string(), value);
        }

        context.set_pin_value("struct_out", json!(result)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: Arc<Board>) {
        node.error = None;

        // Find the output struct pin
        let struct_pin = match node.get_pin_by_name("struct_out") {
            Some(pin) => pin.clone(),
            None => return,
        };

        // Get the connected pin to extract schema
        let connected_pin_id = match struct_pin.connected_to.iter().next() {
            Some(id) => id.clone(),
            None => {
                // No connection - remove generated pins but keep struct_out
                node.pins.retain(|_, pin| pin.pin_type == PinType::Output);
                return;
            }
        };

        let connected_pin = match board.get_pin_by_id(&connected_pin_id) {
            Some(pin) => pin,
            None => {
                node.pins.retain(|_, pin| pin.pin_type == PinType::Output);
                return;
            }
        };

        // Get the schema from the connected pin
        let schema_ref = match &connected_pin.schema {
            Some(s) => s.clone(),
            None => {
                // Check if enforce_schema is true - if so, we need a schema
                if connected_pin
                    .options
                    .as_ref()
                    .is_some_and(|o| o.enforce_schema == Some(true))
                {
                    node.error = Some("Connected pin enforces schema but has none".to_string());
                }
                node.pins.retain(|_, pin| pin.pin_type == PinType::Output);
                return;
            }
        };

        // Schema might be stored as a reference in board.refs - look it up
        let schema_str = board
            .refs
            .get(&schema_ref)
            .cloned()
            .unwrap_or(schema_ref.clone());

        // Parse the JSON schema as a generic Value
        let schema: Value = match flow_like_types::json::from_str(&schema_str) {
            Ok(s) => s,
            Err(e) => {
                node.error = Some(format!("Failed to parse schema: {}", e));
                return;
            }
        };

        // Extract properties from the schema
        let properties = match schema.get("properties").and_then(|p| p.as_object()) {
            Some(props) => props,
            None => {
                node.error = Some("Schema has no object properties".to_string());
                return;
            }
        };

        // Collect the pin names we need for this schema
        let mut relevant_pins = std::collections::HashSet::new();
        relevant_pins.insert("struct_out".to_string());

        // Get required fields
        let required_fields: std::collections::HashSet<&str> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Create input pins for each property (or skip if already exists)
        let mut index = 0u16;
        for (prop_name, prop_schema) in properties {
            let (var_type, value_type) = get_schema_type(prop_schema, &schema);

            // Use a unique prefixed name for the pin to enable special connection rules
            let pin_id = format!("{}{}", MAKE_STRUCT_PIN_PREFIX, prop_name);
            let friendly_name = capitalize_first(prop_name);
            let is_required = required_fields.contains(prop_name.as_str());

            relevant_pins.insert(pin_id.clone());

            // Skip if pin already exists with this name, but also update its schema
            // so that downstream on_update calls can find it (important after paste,
            // where schemas survive the clipboard but may need refreshing).
            if node.pins.iter().any(|(_, p)| p.name == pin_id) {
                if var_type == VariableType::Struct {
                    let standalone = build_standalone_schema(prop_schema, &schema);
                    if let Ok(sub_schema_str) = flow_like_types::json::to_string(&standalone) {
                        if let Some(existing_pin) = node.get_pin_mut_by_name(&pin_id) {
                            existing_pin.schema = Some(sub_schema_str);
                            existing_pin
                                .set_options(PinOptions::new().set_enforce_schema(false).build());
                        }
                    }
                }
                index += 1;
                continue;
            }

            let description = if is_required {
                format!("Field '{}' (required)", prop_name)
            } else {
                format!("Field '{}' (optional)", prop_name)
            };

            let pin = node.add_input_pin(&pin_id, &friendly_name, &description, var_type.clone());
            pin.value_type = value_type.clone();
            pin.index = index;

            // Set default value based on type
            if let Some(default) = get_default_value_for_type(&var_type, &value_type) {
                pin.set_default_value(Some(default));
            }

            // If it's a struct/object type or array, set the sub-schema with definitions
            if var_type == VariableType::Struct {
                let standalone = build_standalone_schema(prop_schema, &schema);
                if let Ok(sub_schema_str) = flow_like_types::json::to_string(&standalone) {
                    pin.schema = Some(sub_schema_str);
                    pin.set_options(PinOptions::new().set_enforce_schema(false).build());
                }
            }

            index += 1;
        }

        // Remove pins that are no longer in the schema
        node.pins.retain(|_, pin| relevant_pins.contains(&pin.name));

        // Update the output pin to have the schema reference
        if let Some(output_pin) = node.get_pin_mut_by_name("struct_out") {
            output_pin.schema = Some(schema_str);
            output_pin.set_options(PinOptions::new().set_enforce_schema(true).build());
        }
    }
}
