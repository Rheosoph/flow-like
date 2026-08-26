use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_unwired_pins},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    json::{Map, json},
};
use std::collections::HashSet;

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

fn is_null_schema(schema: &Value) -> bool {
    schema.get("type").and_then(|t| t.as_str()) == Some("null")
}

fn union_object_properties(schema: &Value, root_schema: &Value) -> Option<Map<String, Value>> {
    let one_of = schema.get("oneOf").and_then(|a| a.as_array())?;
    let mut properties = Map::new();

    for variant in one_of {
        if is_null_schema(variant) {
            continue;
        }

        let resolved = resolve_schema(variant, root_schema);
        if let Some(variant_properties) = resolved.get("properties").and_then(|p| p.as_object()) {
            for (name, prop_schema) in variant_properties {
                properties.insert(name.clone(), prop_schema.clone());
            }
        }
    }

    if properties.is_empty() {
        None
    } else {
        Some(properties)
    }
}

fn add_root_definitions(schema: &mut Value, root_schema: &Value) {
    if let Some(defs) = root_schema.get("definitions") {
        schema["definitions"] = defs.clone();
    } else if let Some(defs) = root_schema.get("$defs") {
        schema["$defs"] = defs.clone();
    }
}

fn is_date_format(schema: &Value) -> bool {
    matches!(
        schema.get("format").and_then(|format| format.as_str()),
        Some("date-time") | Some("date")
    )
}

fn scalar_type(type_str: &str, schema: &Value) -> VariableType {
    match type_str {
        "boolean" => VariableType::Boolean,
        "integer" => VariableType::Integer,
        "number" => VariableType::Float,
        "string" if is_date_format(schema) => VariableType::Date,
        "string" => VariableType::String,
        "object" => VariableType::Struct,
        _ => VariableType::Generic,
    }
}

fn array_type(resolved: &Value, root_schema: &Value) -> (VariableType, ValueType) {
    if let Some(items) = resolved.get("items") {
        let item_resolved = resolve_schema(items, root_schema);
        match item_resolved
            .get("type")
            .and_then(|item_type| item_type.as_str())
        {
            Some(item_type) => (scalar_type(item_type, item_resolved), ValueType::Array),
            None => (VariableType::Struct, ValueType::Array),
        }
    } else {
        (VariableType::Generic, ValueType::Array)
    }
}

/// Get the variable type from a resolved schema
fn get_schema_type(schema: &Value, root_schema: &Value) -> (VariableType, ValueType) {
    let resolved = resolve_schema(schema, root_schema);

    if union_object_properties(resolved, root_schema).is_some() {
        return (VariableType::Struct, ValueType::Normal);
    }

    if let Some(type_val) = resolved.get("type") {
        if let Some(type_str) = type_val.as_str() {
            return match type_str {
                "array" => array_type(resolved, root_schema),
                other => (scalar_type(other, resolved), ValueType::Normal),
            };
        }
        // Nullable types (for example `["string", "null"]`) retain sibling format/items.
        if let Some(types) = type_val.as_array() {
            for t in types {
                if let Some(ts) = t.as_str()
                    && ts != "null"
                {
                    return match ts {
                        "array" => array_type(resolved, root_schema),
                        other => (scalar_type(other, resolved), ValueType::Normal),
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

    if let Some(properties) = union_object_properties(resolved, root_schema) {
        let mut new_schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object"
        });

        new_schema["properties"] = Value::Object(properties);
        add_root_definitions(&mut new_schema, root_schema);

        return new_schema;
    }

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
        add_root_definitions(&mut new_schema, root_schema);

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

/// A producer pin for a `Struct[]` carries the schema one level up from what Break Struct works on.
/// `Get Element`/`For Each` hand over a single item, so unwrap `items` before looking for fields —
/// the same unwrapping [`build_standalone_schema`] already does for nested array properties.
fn unwrap_item_schema<'a>(schema: &'a Value, root_schema: &'a Value) -> &'a Value {
    if schema.get("type").and_then(|t| t.as_str()) != Some("array") {
        return schema;
    }

    match schema.get("items") {
        Some(items) => resolve_schema(items, root_schema),
        None => schema,
    }
}

/// The fields a schema exposes, whether declared directly or spread across a `oneOf` union.
fn object_properties(schema: &Value, root_schema: &Value) -> Option<Map<String, Value>> {
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        return Some(properties.clone());
    }

    union_object_properties(schema, root_schema)
}

/// Drop the field pins the current schema no longer declares — but never one the user has wired.
///
/// A field pin that disappears takes its half of the edge with it, and `fix_pin_connections` then
/// prunes the surviving half on the producer, so the connection vanishes from both ends with no
/// error anywhere. Every schema change would silently cut the wires of every field that moved.
/// `remove_unwired_pins` keeps anything still attached and names it on `node.error` instead, so a
/// mismatch is something the user reads and resolves rather than something they lose.
fn retain_declared_field_pins(node: &mut Node, declared: &HashSet<String>) {
    let stale: Vec<String> = node
        .pins
        .values()
        .filter(|pin| !declared.contains(&pin.name))
        .map(|pin| pin.id.clone())
        .collect();

    remove_unwired_pins(node, &stale);
}

/// Give up on deriving fields: hand `struct_in` its open marker back and drop the unwired pins.
///
/// The marker is what lets any struct producer be wired in. Leaving the last resolved schema on the
/// pin instead would make it a contract: `schemas_are_compatible` rejects two differing concrete
/// schemas, so after unplugging one producer the user could never plug in a different one.
fn reset_to_open(node: &mut Node, error: Option<String>) {
    node.error = error;
    retain_declared_field_pins(node, &HashSet::from(["struct_in".to_string()]));
    if let Some(input_pin) = node.get_pin_mut_by_name("struct_in") {
        input_pin.set_open_schema();
    }
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
        node.set_flowscript_name("struct", "break");
        node.set_receiver("struct_in");
        node.add_icon("/flow/icons/struct.svg");

        // Input struct pin - accepts any struct with a schema
        node.add_input_pin(
            "struct_in",
            "Struct",
            "The struct to break apart",
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let struct_value: Value = context.evaluate_pin("struct_in").await?;

        // Collect output pins first to avoid borrow conflict
        let output_pins: Vec<_> = context
            .node
            .pins
            .iter()
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

    async fn on_update(&self, node: &mut Node, board: &Board) {
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
                reset_to_open(node, None);
                return;
            }
        };

        // A producer that is not on the board handed to us is not evidence that the wire is gone:
        // `node_updates` lifts the node being updated out of the board, and on load this runs
        // before `cleanup` has repaired anything. Dropping the field pins on that incomplete view
        // would delete every wire the user drew from them, so keep them and wait for a full pass.
        let connected_pin = match board.get_pin_by_id(&connected_pin_id) {
            Some(pin) => pin,
            None => return,
        };

        // Get the schema from the connected pin
        let schema_ref = match &connected_pin.schema {
            Some(s) => s.clone(),
            None => {
                reset_to_open(node, Some("Connected struct has no schema".to_string()));
                return;
            }
        };

        // Schema might be stored as a reference in board.refs - look it up
        let schema_str = board
            .refs
            .get(&schema_ref)
            .cloned()
            .unwrap_or(schema_ref.clone());

        if flow_like::flow::pin::is_open_object_schema(&schema_str) {
            reset_to_open(
                node,
                Some(
                    "Connected struct declares no fields. Break Struct needs a producer with a concrete schema."
                        .to_string(),
                ),
            );
            return;
        }

        // Parse the JSON schema as a generic Value
        let schema: Value = match flow_like_types::json::from_str(&schema_str) {
            Ok(s) => s,
            Err(e) => {
                reset_to_open(node, Some(format!("Failed to parse schema: {}", e)));
                return;
            }
        };

        // Resolve the schema in case it has $ref at the top level, then step into the element type
        // when the producer describes an array of structs.
        let resolved_schema = unwrap_item_schema(resolve_schema(&schema, &schema), &schema);

        // Extract properties from the schema
        // JSON Schema stores properties under "properties" key
        let properties = match object_properties(resolved_schema, &schema) {
            Some(props) => props,
            None => {
                // Check if this is a dynamic object type (additionalProperties without properties)
                let error = if resolved_schema.get("additionalProperties").is_some() {
                    "Cannot break dynamic object types (e.g., HashMap). Use a different approach to access the values."
                } else {
                    "Schema has no object properties"
                };
                reset_to_open(node, Some(error.to_string()));
                return;
            }
        };

        // Collect the pin names we need for this schema
        let mut relevant_pins = HashSet::new();
        relevant_pins.insert("struct_in".to_string());

        // Create output pins for each property (or skip if already exists)
        let mut index = 1u16;
        for (prop_name, prop_schema) in &properties {
            let (var_type, value_type) = get_schema_type(prop_schema, &schema);

            // Use a unique prefixed name for the pin to enable special connection rules
            let pin_id = format!("{}{}", BREAK_STRUCT_PIN_PREFIX, prop_name);
            let friendly_name = capitalize_first(prop_name);

            relevant_pins.insert(pin_id.clone());

            // Skip if pin already exists with this name, but also update its schema
            // so that downstream on_update calls can find it (important after paste,
            // where schemas survive the clipboard but may need refreshing).
            if let Some(existing_pin) = node.get_pin_mut_by_name(&pin_id) {
                existing_pin.data_type = var_type.clone();
                existing_pin.value_type = value_type.clone();
                existing_pin.index = index;

                if var_type == VariableType::Struct {
                    let standalone = build_standalone_schema(prop_schema, &schema);
                    if let Ok(sub_schema_str) = flow_like_types::json::to_string(&standalone) {
                        existing_pin.schema = Some(sub_schema_str);
                        existing_pin
                            .set_options(PinOptions::new().set_enforce_schema(false).build());
                    }
                } else {
                    existing_pin.schema = None;
                    existing_pin.options = None;
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

        // Retire the pins this schema no longer declares, keeping any the user still has wired
        retain_declared_field_pins(node, &relevant_pins);

        // Update the input pin to have the schema reference
        if let Some(input_pin) = node.get_pin_mut_by_name("struct_in") {
            input_pin.schema = Some(schema_str);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_and_date_time_formats_become_date_pins() {
        for format in ["date", "date-time"] {
            let schema = json!({ "type": "string", "format": format });
            assert_eq!(
                get_schema_type(&schema, &schema),
                (VariableType::Date, ValueType::Normal)
            );
        }
    }

    #[test]
    fn nullable_and_array_date_time_formats_become_date_pins() {
        let nullable = json!({
            "anyOf": [
                { "type": "null" },
                { "type": "string", "format": "date-time" }
            ]
        });
        assert_eq!(
            get_schema_type(&nullable, &nullable),
            (VariableType::Date, ValueType::Normal)
        );

        let array = json!({
            "type": "array",
            "items": { "type": "string", "format": "date-time" }
        });
        assert_eq!(
            get_schema_type(&array, &array),
            (VariableType::Date, ValueType::Array)
        );
    }
}
