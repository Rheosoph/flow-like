use flow_like::flow::{
    node::{Node, remove_pin},
    pin::PinType,
};
use flow_like_types::Value;
use flow_like_types::json::Map;

/// Signature of a dynamic pin: (name, friendly_name, is_input).
pub type DynamicPinSig = (&'static str, &'static str, bool);

/// Removes dynamic pins that don't match the current operation's expected
/// signatures. Matching pins are left untouched so their ids — and any
/// connections to them — stay stable across board parses.
pub fn retain_dynamic_pins(node: &mut Node, dynamic_names: &[&str], expected: &[DynamicPinSig]) {
    let stale: Vec<_> = node
        .pins
        .values()
        .filter(|p| dynamic_names.contains(&p.name.as_str()))
        .filter(|p| {
            let is_input = p.pin_type == PinType::Input;
            !expected.iter().any(|(name, friendly, input)| {
                p.name == *name && p.friendly_name == *friendly && is_input == *input
            })
        })
        .cloned()
        .collect();
    for pin in stale {
        remove_pin(node, Some(pin));
    }
}

/// Number of existing pins matching a dynamic pin signature.
pub fn count_matching_pins(node: &Node, sig: &DynamicPinSig) -> usize {
    node.pins
        .values()
        .filter(|p| {
            p.name == sig.0 && p.friendly_name == sig.1 && (p.pin_type == PinType::Input) == sig.2
        })
        .count()
}

/// Finds an element in the elements map by ID.
///
/// Supports:
/// - Exact match: "surfaceId/componentId"
/// - Component ID suffix match: "componentId" (matches any "*/componentId")
/// - Page retarget: "otherPageId/componentId" resolves to the current surface's
///   component of the same name, unless the prefix names a widget instance.
pub fn find_element<'a>(
    elements: &'a Map<String, Value>,
    element_id: &str,
) -> Option<(&'a String, &'a Value)> {
    let key = flow_like::a2ui::resolve_element_key(elements, element_id)?;
    elements.get(key).map(|value| (key, value))
}

/// Extracts element ID from either a string or an element object with __element_id field.
/// Used by setter nodes to accept both raw IDs and element refs from Get Element.
pub fn extract_element_id(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(obj) => {
            // Check for __element_id field (set by get_element node)
            obj.get("__element_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                // Fallback to id field
                .or_else(|| {
                    obj.get("id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                })
        }
        Value::Null => None,
        _ => None,
    }
}

/// Get a property from a component's data
pub fn get_component_property<'a>(component: &'a Value, property: &str) -> Option<&'a Value> {
    component.get("component").and_then(|c| c.get(property))
}

/// Get text content from a component (tries multiple common properties)
pub fn get_text_content(component: &Value) -> Option<&str> {
    let comp = component.get("component")?;

    // Try common text properties in order
    comp.get("content")
        .or_else(|| comp.get("text"))
        .or_else(|| comp.get("label"))
        .and_then(|v| v.as_str())
}

/// Get value from a component
pub fn get_value_content(component: &Value) -> Option<&Value> {
    component
        .get("component")
        .and_then(|c| c.get("value").or_else(|| c.get("defaultValue")))
}

/// Extracts element ID from a pin Value that can be either:
/// - A JSON string (from element-select dropdown)
/// - An element object with __element_id (from Get Element node)
///
/// This allows getter nodes to work both when directly selected and when connected to Get Element.
pub fn extract_element_id_from_pin(value: Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s),
        Value::Object(ref obj) => obj
            .get("__element_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                obj.get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            }),
        _ => None,
    }
}
