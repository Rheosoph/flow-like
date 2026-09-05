use super::schema::{
    add_root_definitions, capitalize_first, get_schema_type, resolve_schema, resolve_schema_ref,
    retain_declared_field_pins, union_object_properties,
};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    json::{Map, json},
};
use std::collections::{HashMap, HashSet};

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
        add_root_definitions(&mut new_schema, root_schema);

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
        add_root_definitions(&mut new_schema, root_schema);

        return new_schema;
    }

    // For primitives, return as-is
    resolved.clone()
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

/// Give up on deriving fields: hand `struct_out` its open marker back and drop the unwired pins.
///
/// `on_update` stamps the consumer's schema onto `struct_out` *and* turns on `enforce_schema`.
/// Leaving that behind once the consumer is gone turns the pin into a contract for a shape nothing
/// asked for, and `schemas_are_compatible` would then reject the next consumer the user wires up.
fn reset_to_open(node: &mut Node, error: Option<String>) {
    node.error = error;
    retain_declared_field_pins(node, &HashSet::from(["struct_out".to_string()]));
    if let Some(output_pin) = node.get_pin_mut_by_name("struct_out") {
        output_pin.set_open_schema();
        output_pin.options = None;
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
        node.set_flowscript_name("struct", "makeFromSchema");
        node.add_icon("/flow/icons/struct.svg");

        // Output struct pin - will get schema from connected input
        node.add_output_pin(
            "struct_out",
            "Struct",
            "The constructed struct",
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut result: HashMap<String, Value> = HashMap::new();

        // Get all input pins and build the struct
        for pin in context.node.pins.iter() {
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

    async fn on_update(&self, node: &mut Node, board: &Board) {
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
                reset_to_open(node, None);
                return;
            }
        };

        // A consumer that is not on the board handed to us is not evidence that the wire is gone:
        // `node_updates` lifts the node being updated out of the board, and on load this runs
        // before `cleanup` has repaired anything. Dropping the field pins on that incomplete view
        // would delete every wire the user drew into them, so keep them and wait for a full pass.
        let connected_pin = match board.get_pin_by_id(&connected_pin_id) {
            Some(pin) => pin,
            None => return,
        };

        // Get the schema from the connected pin
        let schema_ref = match &connected_pin.schema {
            Some(s) => s.clone(),
            None => {
                // Check if enforce_schema is true - if so, we need a schema
                let error = connected_pin
                    .options
                    .as_ref()
                    .is_some_and(|o| o.enforce_schema == Some(true))
                    .then(|| "Connected pin enforces schema but has none".to_string());
                reset_to_open(node, error);
                return;
            }
        };

        // Schema might be stored as a reference in board.refs - look it up
        let schema_str = resolve_schema_ref(schema_ref, &board.refs);

        if flow_like::flow::pin::is_open_object_schema(&schema_str) {
            reset_to_open(
                node,
                Some(
                    "Connected struct declares no fields. Make Struct needs a consumer with a concrete schema."
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

        // Extract properties from the schema, resolving a top-level `$ref`/`anyOf` wrapper first
        // so a consumer that declares its shape indirectly reads the same as an inline one.
        let properties: Map<String, Value> = match resolve_schema(&schema, &schema)
            .get("properties")
            .and_then(|p| p.as_object())
        {
            Some(props) => props.clone(),
            None => {
                reset_to_open(node, Some("Schema has no object properties".to_string()));
                return;
            }
        };

        // Collect the pin names we need for this schema
        let mut relevant_pins = HashSet::new();
        relevant_pins.insert("struct_out".to_string());

        // Get required fields
        let required_fields: HashSet<&str> = resolve_schema(&schema, &schema)
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Create input pins for each property (or skip if already exists)
        let mut index = 0u16;
        for (prop_name, prop_schema) in &properties {
            let (var_type, value_type) = get_schema_type(prop_schema, &schema);

            // Use a unique prefixed name for the pin to enable special connection rules
            let pin_id = format!("{}{}", MAKE_STRUCT_PIN_PREFIX, prop_name);
            let friendly_name = capitalize_first(prop_name);
            let is_required = required_fields.contains(prop_name.as_str());

            relevant_pins.insert(pin_id.clone());

            // Skip if pin already exists with this name, but also update its schema
            // so that downstream on_update calls can find it (important after paste,
            // where schemas survive the clipboard but may need refreshing).
            if let Some(existing_pin) = node.get_pin_mut_by_name(&pin_id) {
                existing_pin.data_type = var_type.clone();
                existing_pin.value_type = value_type.clone();
                existing_pin.index = index;

                if existing_pin.default_value.is_none()
                    && let Some(default) = get_default_value_for_type(&var_type, &value_type)
                {
                    existing_pin.set_default_value(Some(default));
                }

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

        // Retire the pins this schema no longer declares, keeping any the user still has wired
        retain_declared_field_pins(node, &relevant_pins);

        // Update the output pin to have the schema reference
        if let Some(output_pin) = node.get_pin_mut_by_name("struct_out") {
            output_pin.schema = Some(schema_str);
            output_pin.set_options(PinOptions::new().set_enforce_schema(true).build());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;

    fn a2ui_style_schema() -> Value {
        flow_like_types::json::to_value(schema_for!(flow_like::a2ui::Style)).unwrap()
    }

    #[test]
    fn a2ui_style_background_schema_is_struct() {
        let schema = a2ui_style_schema();
        let background_schema = schema
            .get("properties")
            .and_then(|properties| properties.get("background"))
            .expect("Style schema should expose background");

        let (var_type, value_type) = get_schema_type(background_schema, &schema);

        assert_eq!(var_type, VariableType::Struct);
        assert_eq!(value_type, ValueType::Normal);
    }

    #[test]
    fn a2ui_style_background_standalone_schema_exposes_variant_fields() {
        let schema = a2ui_style_schema();
        let background_schema = schema
            .get("properties")
            .and_then(|properties| properties.get("background"))
            .expect("Style schema should expose background");

        let standalone = build_standalone_schema(background_schema, &schema);
        let properties = standalone
            .get("properties")
            .and_then(|properties| properties.as_object())
            .expect("background should become an object schema");

        assert!(properties.contains_key("color"));
        assert!(properties.contains_key("gradient"));
        assert!(properties.contains_key("image"));
        assert!(properties.contains_key("blur"));
    }
}
