//! The JSON Schema reader the Structs nodes share.
//!
//! Every node in this family sees the same dialect: schemars output, with `$ref` into
//! `definitions`/`$defs`, an `anyOf` wrapper around each nullable field and `oneOf` for a union.
//! One reader means a schema `Break Struct` can open is one `Cast to Struct` can check.

use flow_like::flow::{
    node::{Node, remove_unwired_pins},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, json::Map};
use std::collections::{HashMap, HashSet};

/// Follow a pin's schema through the board's ref table to the JSON it stands for.
///
/// `Board::cleanup` parks each schema in `board.refs` and leaves a hash key on the pin, so what a
/// pin carries is a key about as often as it is a schema — and a ref can name another ref. The
/// walk is bounded so a table that somehow cycles cannot spin, and a key that is not in the table
/// is handed back untouched: that is the already-expanded case `on_update` normally sees.
pub(crate) fn resolve_schema_ref(schema: String, refs: &HashMap<String, String>) -> String {
    let mut current = schema;
    for _ in 0..8 {
        match refs.get(&current) {
            Some(resolved) => current = resolved.clone(),
            None => break,
        }
    }
    current
}

/// Resolve a `$ref` reference to its definition in the schema.
///
/// `$ref` format: `#/definitions/TypeName` or `#/$defs/TypeName`.
pub(crate) fn resolve_ref<'a>(ref_path: &str, root_schema: &'a Value) -> Option<&'a Value> {
    let path = ref_path.strip_prefix("#/")?;
    let parts: Vec<&str> = path.split('/').collect();

    let mut current = root_schema;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}

/// Resolve a schema that might contain `$ref`, `anyOf`, or be direct.
pub(crate) fn resolve_schema<'a>(schema: &'a Value, root_schema: &'a Value) -> &'a Value {
    if let Some(ref_path) = schema.get("$ref").and_then(|r| r.as_str())
        && let Some(resolved) = resolve_ref(ref_path, root_schema)
    {
        return resolved;
    }

    // `anyOf` is how a nullable field is spelled — take the shape, drop the null.
    if let Some(any_of) = schema.get("anyOf").and_then(|a| a.as_array()) {
        for variant in any_of {
            if variant.get("type").and_then(|t| t.as_str()) == Some("null") {
                continue;
            }
            return resolve_schema(variant, root_schema);
        }
    }

    schema
}

pub(crate) fn is_null_schema(schema: &Value) -> bool {
    schema.get("type").and_then(|t| t.as_str()) == Some("null")
}

/// The fields a `oneOf` union spreads across its variants, flattened into one shape.
pub(crate) fn union_object_properties(
    schema: &Value,
    root_schema: &Value,
) -> Option<Map<String, Value>> {
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

/// The fields a schema exposes, whether declared directly or spread across a `oneOf` union.
pub(crate) fn object_properties(schema: &Value, root_schema: &Value) -> Option<Map<String, Value>> {
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        return Some(properties.clone());
    }

    union_object_properties(schema, root_schema)
}

/// A producer pin for a `Struct[]` carries the schema one level up from what these nodes work on.
/// `Get Element`/`For Each` hand over a single item, so unwrap `items` before looking for fields.
pub(crate) fn unwrap_item_schema<'a>(schema: &'a Value, root_schema: &'a Value) -> &'a Value {
    if schema.get("type").and_then(|t| t.as_str()) != Some("array") {
        return schema;
    }

    match schema.get("items") {
        Some(items) => resolve_schema(items, root_schema),
        None => schema,
    }
}

pub(crate) fn add_root_definitions(schema: &mut Value, root_schema: &Value) {
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

/// Map a scalar JSON-schema type to a pin type (`"format": "date-time"`/`"date"` strings become
/// Date pins).
pub(crate) fn scalar_type(type_str: &str, schema: &Value) -> VariableType {
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

pub(crate) fn array_type(resolved: &Value, root_schema: &Value) -> (VariableType, ValueType) {
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

/// Get the variable type from a resolved schema.
pub(crate) fn get_schema_type(schema: &Value, root_schema: &Value) -> (VariableType, ValueType) {
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
        // Nullable types (for example `["string", "null"]`) retain sibling `format`/`items`.
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

    if resolved.get("properties").is_some() {
        return (VariableType::Struct, ValueType::Normal);
    }

    (VariableType::Generic, ValueType::Normal)
}

pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Drop the field pins the current schema no longer declares — but never one the user has wired.
///
/// A field pin that disappears takes its half of the edge with it, and `fix_pin_connections` then
/// prunes the surviving half on the peer, so the connection vanishes from both ends with no error
/// anywhere. Every schema change would silently cut the wires of every field that moved.
/// `remove_unwired_pins` keeps anything still attached and names it on `node.error` instead, so a
/// mismatch is something the user reads and resolves rather than something they lose.
pub(crate) fn retain_declared_field_pins(node: &mut Node, declared: &HashSet<String>) {
    let stale: Vec<String> = node
        .pins
        .values()
        .filter(|pin| !declared.contains(&pin.name))
        .map(|pin| pin.id.clone())
        .collect();

    remove_unwired_pins(node, &stale);
}

/// How deep a `$ref` chain or a nested value is followed before [`value_matches_schema`] gives up.
/// Schemas can be recursive — a tree node whose child is a tree node — and a cast is a guard, not
/// a proof.
const MAX_DEPTH: usize = 32;

/// Follow `$ref`, and only `$ref`, to the schema it names.
///
/// [`resolve_schema`] also collapses `anyOf` to its first non-null branch, which is what a *pin
/// type* wants: a nullable string is a string pin. Checking a *value* has to see every branch, so
/// it dereferences on its own and handles the unions in [`check`].
fn deref<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    let mut current = schema;
    for _ in 0..MAX_DEPTH {
        let Some(path) = current.get("$ref").and_then(Value::as_str) else {
            return current;
        };
        match resolve_ref(path, root) {
            Some(target) => current = target,
            None => return current,
        }
    }
    current
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_f64() {
                "number"
            } else {
                "integer"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_accepts(declared: &str, value: &Value) -> bool {
    match declared {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        // JSON has one number type, and a whole-numbered float reads back through an integer pin
        // unharmed, so `3.0` is accepted where an integer is declared.
        "integer" => value.as_i64().is_some() || value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        // A dialect keyword we do not model constrains nothing.
        _ => true,
    }
}

fn declared_types(fields: &Map<String, Value>) -> Vec<&str> {
    match fields.get("type") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Whether a value could be read as the shape a schema describes.
///
/// Deliberately lenient: a cast asks whether the reader downstream will find what it needs, not
/// whether the value is exactly this type. Fields the schema never mentions are kept and ignored,
/// `enum`/`format`/`pattern` and the numeric bounds are not policed, and only a field the schema
/// marks `required` has to be present. What *is* checked is what a `Break Struct` would go on to
/// read: the declared fields exist, their primitive types line up, and the nesting matches.
///
/// `root` is the whole schema document, kept apart from `schema` so `$ref` still resolves after
/// the caller has stepped into a sub-schema.
pub(crate) fn value_matches_schema(
    value: &Value,
    schema: &Value,
    root: &Value,
) -> Result<(), String> {
    check(value, schema, root, "struct", 0)
}

fn check(
    value: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Ok(());
    }

    let schema = deref(schema, root);

    match schema {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return Err(format!("{path}: the schema accepts no value")),
        _ => {}
    }

    let Some(fields) = schema.as_object() else {
        return Ok(());
    };

    if let Some(branches) = fields.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            check(value, branch, root, path, depth + 1)?;
        }
    }

    for keyword in ["oneOf", "anyOf"] {
        let Some(branches) = fields.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        if branches.is_empty() {
            continue;
        }

        let mut reasons = Vec::with_capacity(branches.len());
        for branch in branches {
            match check(value, branch, root, path, depth + 1) {
                Ok(()) => return Ok(()),
                Err(reason) => reasons.push(reason),
            }
        }
        return Err(format!(
            "{path}: found {}, which fits none of the {} allowed shapes ({})",
            value_kind(value),
            branches.len(),
            reasons.join("; ")
        ));
    }

    let declared = declared_types(fields);
    let expects_object = declared.iter().any(|name| *name == "object")
        || (declared.is_empty() && fields.contains_key("properties"));

    // A struct pin's schema describes ONE element — a `Struct[]` pin carries the element schema
    // and says `Array` in its value type — so a value that arrives as a list of them is checked
    // element by element against that same schema.
    if expects_object
        && !declared.iter().any(|name| *name == "array")
        && let Some(elements) = value.as_array()
    {
        for (index, element) in elements.iter().enumerate() {
            check(
                element,
                schema,
                root,
                &format!("{path}[{index}]"),
                depth + 1,
            )?;
        }
        return Ok(());
    }

    if !declared.is_empty() && !declared.iter().any(|name| type_accepts(name, value)) {
        return Err(format!(
            "{path}: expected {}, found {}",
            declared.join(" or "),
            value_kind(value)
        ));
    }

    if let Some(properties) = fields.get("properties").and_then(Value::as_object)
        && let Some(object) = value.as_object()
    {
        let required: HashSet<&str> = fields
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        for (name, property) in properties {
            match object.get(name) {
                Some(field) => check(field, property, root, &format!("{path}.{name}"), depth + 1)?,
                None if required.contains(name.as_str()) => {
                    return Err(format!("{path}: missing required field \"{name}\""));
                }
                None => {}
            }
        }
    }

    if let Some(items) = fields.get("items")
        && let Some(elements) = value.as_array()
    {
        for (index, element) in elements.iter().enumerate() {
            check(element, items, root, &format!("{path}[{index}]"), depth + 1)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    fn matches(value: Value, schema: Value) -> Result<(), String> {
        value_matches_schema(&value, &schema, &schema)
    }

    fn person() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" },
                "nickname": { "type": ["string", "null"] }
            },
            "required": ["name", "age"]
        })
    }

    #[test]
    fn extra_fields_are_kept_and_ignored() {
        assert_eq!(
            matches(
                json!({ "name": "Ada", "age": 36, "email": "a@b.c" }),
                person()
            ),
            Ok(())
        );
    }

    #[test]
    fn a_missing_required_field_fails() {
        let error = matches(json!({ "name": "Ada" }), person()).unwrap_err();
        assert!(error.contains("missing required field \"age\""), "{error}");
    }

    #[test]
    fn a_missing_optional_field_is_fine() {
        assert_eq!(
            matches(json!({ "name": "Ada", "age": 36 }), person()),
            Ok(())
        );
    }

    #[test]
    fn a_wrong_primitive_type_fails_and_names_the_field() {
        let error = matches(json!({ "name": "Ada", "age": "36" }), person()).unwrap_err();
        assert!(error.contains("struct.age"), "{error}");
        assert!(error.contains("expected integer"), "{error}");
    }

    #[test]
    fn a_null_on_a_required_field_fails() {
        let error = matches(json!({ "name": null, "age": 36 }), person()).unwrap_err();
        assert!(error.contains("struct.name"), "{error}");
    }

    #[test]
    fn an_explicitly_nullable_field_accepts_null() {
        assert_eq!(
            matches(
                json!({ "name": "Ada", "age": 36, "nickname": null }),
                person()
            ),
            Ok(())
        );
    }

    #[test]
    fn a_whole_numbered_float_satisfies_an_integer() {
        assert_eq!(
            matches(json!({ "name": "Ada", "age": 36.0 }), person()),
            Ok(())
        );
    }

    #[test]
    fn a_fractional_number_does_not_satisfy_an_integer() {
        assert!(matches(json!({ "name": "Ada", "age": 36.5 }), person()).is_err());
    }

    /// A struct pin's schema is per element, so a value that arrives as a list is checked one
    /// element at a time against it.
    #[test]
    fn an_array_is_checked_against_the_element_schema() {
        assert_eq!(
            matches(
                json!([{ "name": "Ada", "age": 36 }, { "name": "Grace", "age": 45 }]),
                person()
            ),
            Ok(())
        );

        let error = matches(
            json!([{ "name": "Ada", "age": 36 }, { "name": "Grace" }]),
            person(),
        )
        .unwrap_err();
        assert!(error.contains("struct[1]"), "{error}");
    }

    #[test]
    fn a_non_object_never_passes_for_an_object_schema() {
        let error = matches(json!("Ada"), person()).unwrap_err();
        assert!(error.contains("expected object, found string"), "{error}");
    }

    #[test]
    fn nested_objects_are_walked() {
        let schema = json!({
            "type": "object",
            "properties": {
                "owner": {
                    "type": "object",
                    "properties": { "id": { "type": "integer" } },
                    "required": ["id"]
                }
            },
            "required": ["owner"]
        });

        assert_eq!(
            matches(json!({ "owner": { "id": 1 } }), schema.clone()),
            Ok(())
        );
        let error = matches(json!({ "owner": { "id": "1" } }), schema).unwrap_err();
        assert!(error.contains("struct.owner.id"), "{error}");
    }

    #[test]
    fn refs_into_definitions_resolve() {
        let schema = json!({
            "type": "object",
            "properties": { "owner": { "$ref": "#/$defs/Owner" } },
            "required": ["owner"],
            "$defs": {
                "Owner": {
                    "type": "object",
                    "properties": { "id": { "type": "integer" } },
                    "required": ["id"]
                }
            }
        });

        assert_eq!(
            matches(json!({ "owner": { "id": 1 } }), schema.clone()),
            Ok(())
        );
        assert!(matches(json!({ "owner": {} }), schema).is_err());
    }

    #[test]
    fn a_union_passes_when_any_branch_fits() {
        let schema = json!({
            "oneOf": [
                { "type": "object", "properties": { "color": { "type": "string" } }, "required": ["color"] },
                { "type": "object", "properties": { "blur": { "type": "number" } }, "required": ["blur"] }
            ]
        });

        assert_eq!(matches(json!({ "blur": 4 }), schema.clone()), Ok(()));
        let error = matches(json!({ "gradient": true }), schema).unwrap_err();
        assert!(
            error.contains("fits none of the 2 allowed shapes"),
            "{error}"
        );
    }

    #[test]
    fn array_properties_are_checked_per_item() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array", "items": { "type": "string" } }
            }
        });

        assert_eq!(
            matches(json!({ "tags": ["a", "b"] }), schema.clone()),
            Ok(())
        );
        let error = matches(json!({ "tags": ["a", 2] }), schema).unwrap_err();
        assert!(error.contains("struct.tags[1]"), "{error}");
    }

    #[test]
    fn an_open_object_schema_accepts_anything_object_shaped() {
        let schema = json!({ "type": "object", "additionalProperties": true });
        assert_eq!(matches(json!({ "anything": [1, 2] }), schema), Ok(()));
    }

    #[test]
    fn enums_and_formats_are_not_policed() {
        let schema = json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["read", "write"] },
                "created": { "type": "string", "format": "date-time" }
            }
        });

        assert_eq!(
            matches(json!({ "mode": "delete", "created": "yesterday" }), schema),
            Ok(())
        );
    }

    #[test]
    fn a_recursive_schema_terminates() {
        let schema = json!({
            "$ref": "#/$defs/Tree",
            "$defs": {
                "Tree": {
                    "type": "object",
                    "properties": { "child": { "$ref": "#/$defs/Tree" } }
                }
            }
        });

        let mut value = json!({ "child": {} });
        for _ in 0..80 {
            value = json!({ "child": value });
        }

        assert_eq!(matches(value, schema), Ok(()));
    }
}
