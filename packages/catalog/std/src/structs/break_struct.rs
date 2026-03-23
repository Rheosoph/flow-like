use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

/// Unique identifier prefix for break struct pins to enable special connection rules
pub const BREAK_STRUCT_PIN_PREFIX: &str = "__break_struct_field__";

#[crate::register_node]
#[derive(Default)]
pub struct BreakStructNode {}

impl BreakStructNode {
    pub fn new() -> Self {
        BreakStructNode {}
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
        || resolved.get("additionalProperties").is_some()
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
        // Preserve additionalProperties for dynamic object types (e.g., HashMap)
        if let Some(additional) = resolved.get("additionalProperties") {
            new_schema["additionalProperties"] = additional.clone();
        }
        if let Some(defs) = root_schema.get("definitions") {
            new_schema["definitions"] = defs.clone();
        } else if let Some(defs) = root_schema.get("$defs") {
            new_schema["$defs"] = defs.clone();
        }

        return new_schema;
    }

    // For arrays, extract the item schema (not the array schema itself)
    // This is because Break Struct works on single items, and For Each will iterate
    // providing individual items, so the schema should be the item type
    if resolved.get("type").and_then(|t| t.as_str()) == Some("array") {
        if let Some(items) = resolved.get("items") {
            // Recursively build standalone schema for the items type
            return build_standalone_schema(items, root_schema);
        }
        // No items schema, return empty object schema
        return json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object"
        });
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

#[async_trait]
impl NodeLogic for BreakStructNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_break",
            "Break Struct",
            "Breaks a struct into its individual fields based on the schema",
            "Structs",
        );
        node.add_icon("/flow/icons/struct.svg");

        // Input struct pin - accepts any struct with a schema
        node.add_input_pin(
            "struct_in",
            "Struct",
            "The struct to break apart",
            VariableType::Struct,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let struct_value: Value = context.evaluate_pin("struct_in").await?;

        // Collect output pins first to avoid borrow conflict
        let output_pins: Vec<_> = context
            .node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Output)
            .cloned()
            .collect();

        // Get all output pins and extract their field values from the struct
        for pin in output_pins {
            let pin_name = &pin.name;

            // Extract field name from the prefixed pin name
            let field_name = pin_name
                .strip_prefix(BREAK_STRUCT_PIN_PREFIX)
                .unwrap_or(pin_name);

            let field_value = struct_value.get(field_name).cloned().unwrap_or(Value::Null);

            context.set_pin_ref_value(&pin, field_value).await?;
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: Arc<Board>) {
        node.error = None;

        // Find the input struct pin
        let struct_pin = match node.get_pin_by_name("struct_in") {
            Some(pin) => pin.clone(),
            None => return,
        };

        // Get the connected pin to extract schema
        let connected_pin_id = match struct_pin.depends_on.iter().next() {
            Some(id) => id.clone(),
            None => {
                // No connection - remove generated pins but keep struct_in
                node.pins.retain(|_, pin| pin.pin_type == PinType::Input);
                return;
            }
        };

        let connected_pin = match board.get_pin_by_id(&connected_pin_id) {
            Some(pin) => pin,
            None => {
                node.pins.retain(|_, pin| pin.pin_type == PinType::Input);
                return;
            }
        };

        // Get the schema from the connected pin
        let schema_ref = match &connected_pin.schema {
            Some(s) => s.clone(),
            None => {
                node.error = Some("Connected struct has no schema".to_string());
                node.pins.retain(|_, pin| pin.pin_type == PinType::Input);
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

        // Resolve the schema in case it has $ref at the top level
        let resolved_schema = resolve_schema(&schema, &schema);

        // Extract properties from the schema
        // JSON Schema stores properties under "properties" key
        let properties = match resolved_schema
            .get("properties")
            .and_then(|p| p.as_object())
        {
            Some(props) => props,
            None => {
                // Check if this is a dynamic object type (additionalProperties without properties)
                if resolved_schema.get("additionalProperties").is_some() {
                    node.error = Some("Cannot break dynamic object types (e.g., HashMap). Use a different approach to access the values.".to_string());
                } else {
                    node.error = Some("Schema has no object properties".to_string());
                }
                node.pins.retain(|_, pin| pin.pin_type == PinType::Input);
                return;
            }
        };

        // Collect the pin names we need for this schema
        let mut relevant_pins = std::collections::HashSet::new();
        relevant_pins.insert("struct_in".to_string());

        // Create output pins for each property (or skip if already exists)
        let mut index = 1u16;
        for (prop_name, prop_schema) in properties {
            let (var_type, value_type) = get_schema_type(prop_schema, &schema);

            // Use a unique prefixed name for the pin to enable special connection rules
            let pin_id = format!("{}{}", BREAK_STRUCT_PIN_PREFIX, prop_name);
            let friendly_name = capitalize_first(prop_name);

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

            let pin = node.add_output_pin(
                &pin_id,
                &friendly_name,
                &format!("Field '{}' from the struct", prop_name),
                var_type.clone(),
            );
            pin.value_type = value_type;
            pin.index = index;

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

        // Update the input pin to have the schema reference
        if let Some(input_pin) = node.get_pin_mut_by_name("struct_in") {
            input_pin.schema = Some(schema_str);
        }
    }
}
