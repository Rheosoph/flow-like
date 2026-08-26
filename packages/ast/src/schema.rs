//! JSON Schema ⇄ FlowScript interface helpers.
//!
//! The board model still stores schemas as JSON Schema strings. Interfaces are the text-domain
//! authoring form; parsing an interface generates the same schema string that older `@schema`
//! decorators carried.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::model::{InterfaceDecl, InterfaceField, InterfaceType, Literal, VarDecl};

/// Normalize a schema string into compact canonical JSON. Object key order is canonical because
/// `serde_json::Map` is backed by an ordered map in this workspace configuration.
pub fn normalize_schema(schema: &str) -> Option<String> {
    let value: Value = serde_json::from_str(schema).ok()?;
    serde_json::to_string(&value).ok()
}

/// Normalize only real JSON object schemas.
///
/// Some board schemas can arrive as unresolved numeric/string handles. Those are valid JSON
/// values, but they are not useful FlowScript surface syntax and should not render as `@schema`.
pub fn normalize_object_schema(schema: &str) -> Option<String> {
    let value: Value = serde_json::from_str(schema).ok()?;
    value.as_object()?;
    serde_json::to_string(&value).ok()
}

/// Build a compact JSON Schema string for an interface declaration.
pub fn schema_from_interface(interface: &InterfaceDecl) -> Option<String> {
    schema_from_interface_with_defs(interface, &[])
}

/// Build a compact JSON Schema string for an interface declaration, including referenced
/// interfaces in `$defs`.
pub fn schema_from_interface_with_defs(
    interface: &InterfaceDecl,
    interfaces: &[InterfaceDecl],
) -> Option<String> {
    let mut refs = BTreeSet::new();
    for field in &interface.fields {
        collect_type_refs(&field.ty, &mut refs);
    }
    refs.remove(&interface.name);

    let mut schema = interface_schema_value(interface, false)?;
    let known: HashMap<&str, &InterfaceDecl> = interfaces
        .iter()
        .map(|decl| (decl.name.as_str(), decl))
        .collect();

    let mut defs = Map::new();
    let mut visited = BTreeSet::new();
    while let Some(name) = refs.pop_first() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(decl) = known.get(name.as_str()).copied() else {
            continue;
        };
        for field in &decl.fields {
            collect_type_refs(&field.ty, &mut refs);
        }
        refs.remove(&interface.name);
        let def = interface_schema_value(decl, false)?;
        defs.insert(name, def);
    }

    if !defs.is_empty() {
        schema
            .as_object_mut()?
            .insert("$defs".to_string(), Value::Object(defs));
    }

    serde_json::to_string(&schema).ok()
}

/// Generate interface declarations for schema-bearing variables.
///
/// The interface text is the readable structural view. When the board supplied a richer JSON
/// Schema (for example with `$schema`, `format`, `title`, or validation bounds), the root
/// interface keeps that exact schema internally so variable rendering can still map the board
/// schema to the interface name without exposing a giant `@schema(...)` decorator.
pub fn interfaces_for_variables(vars: &[VarDecl]) -> Vec<InterfaceDecl> {
    let mut interfaces = Vec::new();
    let mut seen_schemas = HashSet::new();
    let mut used_names = HashSet::new();

    for var in vars {
        let Some(schema) = &var.schema else {
            continue;
        };
        let Some(normalized) = normalize_schema(schema) else {
            continue;
        };
        if !seen_schemas.insert(normalized.clone()) {
            continue;
        }

        let hint = pascal_case(&var.name);
        let Some(mut decls) = interfaces_from_schema(&normalized, &hint) else {
            continue;
        };

        // Dedup collisions against earlier variables' interfaces. Renames must also be
        // applied to the `Named` references within this schema family, otherwise a
        // renamed `$defs` interface is declared under the new name while fields keep
        // pointing at the old one — which now belongs to a different schema.
        let mut renames: HashMap<String, String> = HashMap::new();
        for decl in &mut decls {
            let base = decl.name.clone();
            decl.name = unique_name(&base, &mut used_names);
            if decl.name != base {
                renames.insert(base, decl.name.clone());
            }
        }
        for decl in &mut decls {
            if !renames.is_empty() {
                for field in &mut decl.fields {
                    rename_type_refs(&mut field.ty, &renames);
                }
            }
            if decl.schema.is_none() {
                decl.schema = schema_from_interface(decl);
            }
        }

        interfaces.extend(decls);
    }

    interfaces
}

/// Return the interface name that represents `schema`, if any.
pub fn interface_name_for_schema<'a>(
    interfaces: &'a [InterfaceDecl],
    schema: &str,
) -> Option<&'a str> {
    let normalized = normalize_schema(schema)?;
    interfaces.iter().find_map(|decl| {
        let decl_schema = decl.schema.as_deref()?;
        json_equal(&normalized, decl_schema).then_some(decl.name.as_str())
    })
}

/// After parsing, attach generated schemas to variables whose type refers to an interface.
pub fn apply_interface_schemas(ast: &mut crate::model::BoardAst) {
    let interfaces = ast.interfaces.clone();
    for decl in &mut ast.interfaces {
        decl.schema = schema_from_interface_with_defs(decl, &interfaces);
    }

    let schema_by_name: HashMap<String, String> = ast
        .interfaces
        .iter()
        .filter_map(|decl| Some((decl.name.clone(), decl.schema.clone()?)))
        .collect();

    for var in &mut ast.variables {
        if var.schema.is_none()
            && let Some(schema) = schema_by_name.get(&var.ty.base)
        {
            var.schema = Some(schema.clone());
            var.ty.base = "Struct".to_string();
        }
    }
}

fn interfaces_from_schema(schema: &str, name_hint: &str) -> Option<Vec<InterfaceDecl>> {
    let value: Value = serde_json::from_str(schema).ok()?;
    let defs = value.get("$defs").and_then(Value::as_object);
    let mut decls = Vec::new();

    if let Some(defs) = defs {
        for (name, def) in defs {
            if is_object_schema(def) {
                decls.push(interface_from_object_schema(name, def, defs)?);
            }
        }
    }

    if !is_object_schema(&value) {
        return None;
    }
    let root_name = value
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| name_hint.to_string());
    let mut root = interface_from_object_schema(&root_name, &value, defs.unwrap_or(&Map::new()))?;
    root.schema = normalize_schema(schema);
    decls.push(root);
    Some(decls)
}

fn is_object_schema(value: &Value) -> bool {
    value.get("properties").and_then(Value::as_object).is_some()
        && value
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|ty| ty == "object")
}

fn interface_from_object_schema(
    name: &str,
    schema: &Value,
    defs: &Map<String, Value>,
) -> Option<InterfaceDecl> {
    // `oneOf`/`allOf` carry constraints that the interface syntax cannot currently express.
    for unsupported in ["oneOf", "allOf"] {
        if schema.get(unsupported).is_some() {
            return None;
        }
    }

    let properties = schema.get("properties")?.as_object()?;
    let required: BTreeSet<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut fields = Vec::new();
    for (field_name, property) in properties {
        let ty = interface_type_from_schema(property, defs)?;
        fields.push(InterfaceField {
            name: field_name.clone(),
            ty,
            optional: !required.contains(field_name),
            default: property.get("default").and_then(literal_from_json),
        });
    }

    Some(InterfaceDecl {
        name: pascal_case(name),
        fields,
        schema: None,
    })
}

fn interface_type_from_schema(schema: &Value, defs: &Map<String, Value>) -> Option<InterfaceType> {
    if schema == &Value::Bool(true) {
        return Some(InterfaceType::Any);
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.strip_prefix("#/$defs/")?;
        if let Some(def) = defs.get(name) {
            if is_object_schema(def) {
                return Some(InterfaceType::Named(pascal_case(name)));
            }
            if let Some(ty) = interface_type_from_schema(def, defs) {
                return Some(ty);
            }
        }
        return Some(InterfaceType::Named(pascal_case(name)));
    }
    if let Some(enum_ty) = enum_type_from_schema(schema) {
        return Some(enum_ty);
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
        let mut members = Vec::new();
        for item in any_of {
            members.push(interface_type_from_schema(item, defs)?);
        }
        return Some(union_type(members));
    }
    if let Some(types) = schema.get("type").and_then(Value::as_array) {
        let mut members = Vec::new();
        for item in types {
            let ty = item.as_str()?;
            let mut variant = schema.clone();
            variant
                .as_object_mut()?
                .insert("type".to_string(), Value::String(ty.to_string()));
            members.push(interface_type_from_schema(&variant, defs)?);
        }
        return Some(union_type(members));
    }
    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        return match ty {
            "array" => {
                let item = schema.get("items").unwrap_or(&Value::Bool(true));
                Some(InterfaceType::Array(Box::new(interface_type_from_schema(
                    item, defs,
                )?)))
            }
            "object" => {
                if let Some(additional) = schema.get("additionalProperties") {
                    return Some(InterfaceType::Map(Box::new(interface_type_from_schema(
                        additional, defs,
                    )?)));
                }
                Some(InterfaceType::Named("Struct".to_string()))
            }
            "string" if is_date_schema(schema) => Some(InterfaceType::Named("Date".to_string())),
            _ => primitive_schema_type(ty),
        };
    }
    if is_object_schema(schema) {
        return Some(InterfaceType::Named("Struct".to_string()));
    }
    Some(InterfaceType::Any)
}

fn enum_type_from_schema(schema: &Value) -> Option<InterfaceType> {
    let values = schema.get("enum")?.as_array()?;
    let mut members = Vec::new();
    for value in values {
        members.push(match value {
            Value::String(value) => InterfaceType::StringLiteral(value.clone()),
            Value::Null => InterfaceType::Null,
            _ => return None,
        });
    }
    Some(union_type(members))
}

fn primitive_schema_type(ty: &str) -> Option<InterfaceType> {
    Some(match ty {
        "string" => InterfaceType::Named("string".to_string()),
        "integer" => InterfaceType::Named("int".to_string()),
        "number" => InterfaceType::Named("float".to_string()),
        "boolean" => InterfaceType::Named("bool".to_string()),
        "null" => InterfaceType::Null,
        _ => return None,
    })
}

fn is_date_schema(schema: &Value) -> bool {
    matches!(
        schema.get("format").and_then(Value::as_str),
        Some("date") | Some("date-time")
    )
}

fn union_type(mut members: Vec<InterfaceType>) -> InterfaceType {
    members.dedup();
    if members.len() == 1 {
        members.pop().unwrap()
    } else {
        InterfaceType::Union(members)
    }
}

fn interface_schema_value(interface: &InterfaceDecl, include_title: bool) -> Option<Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();

    for field in &interface.fields {
        let mut field_schema = schema_value_from_type(&field.ty)?;
        if let Some(default) = &field.default {
            // `any` maps to the boolean schema `true`; upgrade it to its object
            // equivalent `{}` so the default can attach instead of failing the
            // whole interface (which would wipe the variable's schema).
            if !field_schema.is_object() {
                field_schema = Value::Object(Map::new());
            }
            field_schema
                .as_object_mut()?
                .insert("default".to_string(), literal_to_json(default));
        }
        if !field.optional {
            required.push(Value::String(field.name.clone()));
        }
        properties.insert(field.name.clone(), field_schema);
    }

    let mut object = Map::new();
    if include_title {
        object.insert("title".to_string(), Value::String(interface.name.clone()));
    }
    object.insert("type".to_string(), Value::String("object".to_string()));
    object.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        object.insert("required".to_string(), Value::Array(required));
    }
    Some(Value::Object(object))
}

fn schema_value_from_type(ty: &InterfaceType) -> Option<Value> {
    match ty {
        InterfaceType::Named(name) => match name.as_str() {
            "string" => Some(json!({ "type": "string" })),
            "Date" => Some(json!({ "type": "string", "format": "date-time" })),
            "int" => Some(json!({ "type": "integer" })),
            "float" | "number" => Some(json!({ "type": "number" })),
            "bool" | "boolean" => Some(json!({ "type": "boolean" })),
            "Struct" => Some(json!({ "type": "object" })),
            other => Some(json!({ "$ref": format!("#/$defs/{other}") })),
        },
        InterfaceType::Array(inner) => {
            Some(json!({ "type": "array", "items": schema_value_from_type(inner)? }))
        }
        InterfaceType::Map(inner) => Some(
            json!({ "type": "object", "additionalProperties": schema_value_from_type(inner)? }),
        ),
        InterfaceType::Set(inner) => Some(
            json!({ "type": "array", "items": schema_value_from_type(inner)?, "uniqueItems": true }),
        ),
        InterfaceType::Union(members) => {
            if members.iter().all(|member| {
                matches!(
                    member,
                    InterfaceType::StringLiteral(_) | InterfaceType::Null
                )
            }) {
                let values: Vec<Value> = members
                    .iter()
                    .map(|member| match member {
                        InterfaceType::StringLiteral(value) => Value::String(value.clone()),
                        InterfaceType::Null => Value::Null,
                        _ => unreachable!(),
                    })
                    .collect();
                return Some(json!({ "enum": values }));
            }

            let simple_types: Option<Vec<Value>> = members
                .iter()
                .map(simple_json_type_name)
                .collect::<Option<Vec<_>>>();
            if let Some(types) = simple_types {
                return Some(json!({ "type": types }));
            }

            let any_of = members
                .iter()
                .map(schema_value_from_type)
                .collect::<Option<Vec<_>>>()?;
            Some(json!({ "anyOf": any_of }))
        }
        InterfaceType::StringLiteral(value) => Some(json!({ "enum": [value] })),
        InterfaceType::Null => Some(json!({ "type": "null" })),
        InterfaceType::Any => Some(Value::Bool(true)),
    }
}

fn simple_json_type_name(ty: &InterfaceType) -> Option<Value> {
    let name = match ty {
        InterfaceType::Named(name) if name == "string" => "string",
        InterfaceType::Named(name) if name == "int" => "integer",
        InterfaceType::Named(name) if name == "float" || name == "number" => "number",
        InterfaceType::Named(name) if name == "bool" || name == "boolean" => "boolean",
        InterfaceType::Null => "null",
        _ => return None,
    };
    Some(Value::String(name.to_string()))
}

fn rename_type_refs(ty: &mut InterfaceType, renames: &HashMap<String, String>) {
    match ty {
        InterfaceType::Named(name) => {
            if let Some(renamed) = renames.get(name) {
                *name = renamed.clone();
            }
        }
        InterfaceType::Array(inner) | InterfaceType::Map(inner) | InterfaceType::Set(inner) => {
            rename_type_refs(inner, renames)
        }
        InterfaceType::Union(members) => {
            for member in members {
                rename_type_refs(member, renames);
            }
        }
        _ => {}
    }
}

fn collect_type_refs(ty: &InterfaceType, refs: &mut BTreeSet<String>) {
    match ty {
        InterfaceType::Named(name)
            if !matches!(
                name.as_str(),
                "string" | "int" | "float" | "number" | "bool" | "boolean" | "Date" | "Struct"
            ) =>
        {
            refs.insert(name.clone());
        }
        InterfaceType::Array(inner) | InterfaceType::Map(inner) | InterfaceType::Set(inner) => {
            collect_type_refs(inner, refs)
        }
        InterfaceType::Union(members) => {
            for member in members {
                collect_type_refs(member, refs);
            }
        }
        _ => {}
    }
}

fn literal_to_json(lit: &Literal) -> Value {
    match lit {
        Literal::String(value) => Value::String(value.clone()),
        Literal::Int(value) => Value::from(*value),
        Literal::Float(value) => Value::from(*value),
        Literal::Bool(value) => Value::Bool(*value),
        Literal::Null => Value::Null,
        Literal::Json(raw) => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
        }
    }
}

fn literal_from_json(value: &Value) -> Option<Literal> {
    Some(match value {
        Value::String(value) => Literal::String(value.clone()),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Literal::Int(value)
            } else {
                Literal::Float(value.as_f64()?)
            }
        }
        Value::Bool(value) => Literal::Bool(*value),
        Value::Null => Literal::Null,
        Value::Array(_) | Value::Object(_) => Literal::Json(serde_json::to_string(value).ok()?),
    })
}

fn json_equal(lhs: &str, rhs: &str) -> bool {
    let Ok(lhs) = serde_json::from_str::<Value>(lhs) else {
        return false;
    };
    let Ok(rhs) = serde_json::from_str::<Value>(rhs) else {
        return false;
    };
    lhs == rhs
}

fn pascal_case(input: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for c in input.chars() {
        // Interface names must lex as identifiers; every non-alphanumeric char
        // (`.`, `/`, `:` … not just `_`/`-`/space) acts as a word separator.
        if !c.is_alphanumeric() {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        "StructShape".to_string()
    } else if out.chars().next().is_some_and(|c| c.is_numeric()) {
        format!("_{out}")
    } else {
        out
    }
}

fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut i = 2;
    loop {
        let candidate = format!("{base}{i}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        i += 1;
    }
}
