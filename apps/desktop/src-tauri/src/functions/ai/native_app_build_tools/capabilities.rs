use super::*;
use std::collections::HashSet;
fn column_valid(column: &Value) -> bool {
    (column["type"] == "vector") == column.get("vector_size").is_some()
}
fn sample_value(field: &Value, value: &Value) -> bool {
    if value.is_null() {
        return field["nullable"] == true;
    }
    let kind = text(field, "type");
    if ["string", "binary", "date32", "timestamp:ms:UTC"].contains(&kind) {
        return value.is_string();
    }
    if kind == "boolean" {
        return value.is_boolean();
    }
    if kind == "vector" {
        return value.as_array().is_some_and(|values| {
            Some(values.len() as u64) == field["vector_size"].as_u64()
                && values
                    .iter()
                    .all(|v| v.as_f64().is_some_and(f64::is_finite))
        });
    }
    value.as_f64().is_some_and(|n| {
        n.is_finite()
            && (!kind.contains("int") || n.fract() == 0.0)
            && (!kind.contains("uint") || n >= 0.0)
    })
}
fn fields_valid(
    fields: &Value,
    sample: &Value,
    reserved: &[&str],
    require_all: bool,
) -> Result<(), String> {
    let fields = fields
        .as_array()
        .ok_or("Capability fields must be an array.")?;
    let mut seen = HashSet::new();
    for field in fields {
        let name = text(field, "name");
        if !seen.insert(name) || reserved.contains(&name) || !column_valid(field) {
            return Err(format!(
                "Capability field '{name}' is duplicated, reserved, or has an invalid vector size."
            ));
        }
        if let Some(value) = sample.get(name) {
            if !sample_value(field, value) {
                return Err(format!("Sample field '{name}' has the wrong type."));
            }
        } else if require_all && field["nullable"] != true {
            return Err(format!("Sample is missing required field '{name}'."));
        }
    }
    for name in sample
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| name)
    {
        if !seen.contains(name.as_str()) {
            return Err(format!("Sample field '{name}' is not declared."));
        }
    }
    Ok(())
}
fn substitute(value: &mut Value, replacements: &[(String, String)]) {
    match value {
        Value::String(text) => {
            let source = text.clone();
            let mut output = String::with_capacity(source.len());
            let mut rest = source.as_str();
            while !rest.is_empty() {
                if let Some((from, to)) =
                    replacements.iter().find(|(from, _)| rest.starts_with(from))
                {
                    output.push_str(to);
                    rest = &rest[from.len()..];
                } else {
                    let next = rest.chars().next().unwrap();
                    output.push(next);
                    rest = &rest[next.len_utf8()..];
                }
            }
            *text = output;
        }
        Value::Array(values) => {
            for value in values {
                substitute(value, replacements)
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                substitute(value, replacements)
            }
        }
        _ => {}
    }
}
pub(super) fn instantiate(id: &str, version: &str, parameters: &Value) -> Result<Value, String> {
    if version != "1.0.0" || !matches!(id, "crud.foundation" | "approval.foundation") {
        return Err(format!(
            "Unknown app capability {id}@{version}. Use an exact advertised version."
        ));
    }
    let mut normalized = parameters.clone();
    for key in [
        "namespace",
        "page_name",
        "route",
        "entity_name",
        "table_name",
        "request_name",
        "requests_table_name",
        "audit_table_name",
        "requester_id",
        "approver_id",
    ] {
        if let Some(value) = normalized[key].as_str() {
            normalized[key] = json!(value.trim());
        }
    }
    for kind in ["fields", "request_fields"] {
        for column in normalized
            .get_mut(kind)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(name) = column["name"].as_str() {
                column["name"] = json!(name.trim());
            }
        }
    }
    let validator = jsonschema::validator_for(&contract()["parameter_schemas"][id])
        .map_err(|e| e.to_string())?;
    if let Some(error) = validator.iter_errors(&normalized).next() {
        return Err(format!("Invalid capability parameters: {error}"));
    }
    if id == "crud.foundation" {
        fields_valid(
            &normalized["fields"],
            &normalized["sample_record"],
            &["id", "created_at", "updated_at"],
            true,
        )?;
        fields_valid(
            &normalized["fields"],
            &normalized["sample_update"],
            &["id", "created_at", "updated_at"],
            false,
        )?;
        if normalized["sample_update"]
            .as_object()
            .is_none_or(|o| o.is_empty())
        {
            return Err("sample_update must change at least one declared field.".into());
        }
    } else {
        if normalized["requests_table_name"] == normalized["audit_table_name"] {
            return Err("Requests and audit tables must be distinct.".into());
        }
        fields_valid(
            &normalized["request_fields"],
            &normalized["sample_request"],
            &[
                "id",
                "status",
                "requester_id",
                "submitted_at",
                "updated_at",
                "version",
            ],
            true,
        )?;
    }
    let template_parameters = &contract()["template_parameters"][id];
    let namespace = text(&normalized, "namespace");
    let command_namespace = namespace
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let mut replacements = vec![("native_template_ns".into(), command_namespace)];
    for (key, placeholder) in template_parameters.as_object().into_iter().flatten() {
        if let Some(placeholder) = placeholder.as_str() {
            replacements.push((placeholder.to_owned(), text(&normalized, key).to_owned()));
        }
    }
    replacements.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    let mut fragment = contract()["templates"][id].clone();
    substitute(&mut fragment, &replacements);
    fragment["capability"]["parameters"] = normalized.clone();
    let columns = &mut fragment["resources"][0]["config"]["columns"];
    let columns = columns
        .as_array_mut()
        .ok_or("Invalid bundled capability table template.")?;
    columns.retain(|column| column["name"] != "native_field");
    columns.extend(
        array(
            &normalized,
            if id == "crud.foundation" {
                "fields"
            } else {
                "request_fields"
            },
        )
        .iter()
        .cloned(),
    );
    let lifecycle = &mut fragment["scenarios"][1];
    if id == "crud.foundation" {
        let record = &mut lifecycle["steps"][0]["arguments"]["payload"]["record"];
        record.as_object_mut().unwrap().remove("native_field");
        record
            .as_object_mut()
            .unwrap()
            .extend(normalized["sample_record"].as_object().unwrap().clone());
        lifecycle["steps"][2]["arguments"]["payload"]["changes"] =
            normalized["sample_update"].clone();
        let assertions = lifecycle["assertions"].as_array_mut().unwrap();
        let slot = assertions
            .iter()
            .position(|a| a["id"] == "update_record_native_field_updated")
            .ok_or("Invalid bundled update assertion template.")?;
        assertions.remove(slot);
        let mut update_fields = normalized["sample_update"]
            .as_object()
            .unwrap()
            .iter()
            .collect::<Vec<_>>();
        update_fields.sort_by(|(left, _), (right, _)| compare(left, right));
        let updated = update_fields.into_iter().map(|(field, expected)| {
            let safe = field.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect::<String>();
            json!({"id":format!("update_record_{safe}_updated"),"kind":"state","step_id":"update","path":format!("/record/{}",field.replace('~',"~0").replace('/',"~1")),"operator":"equals","expected":expected})
        }).collect::<Vec<_>>();
        assertions.splice(slot..slot, updated);
    } else {
        let request = &mut lifecycle["steps"][0]["arguments"]["payload"]["request"];
        request.as_object_mut().unwrap().remove("native_field");
        request
            .as_object_mut()
            .unwrap()
            .extend(normalized["sample_request"].as_object().unwrap().clone());
    }
    Ok(fragment)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capabilities_match_web_templates() {
        for vector in array(contract(), "recipe_vectors") {
            assert_eq!(
                instantiate(
                    text(vector, "id"),
                    text(vector, "version"),
                    &vector["parameters"]
                )
                .unwrap(),
                vector["output"]
            );
        }
        for (id, parameters) in contract()["template_parameters"].as_object().unwrap() {
            assert_eq!(
                instantiate(id, "1.0.0", parameters).unwrap(),
                contract()["templates"][id]
            );
        }
    }
    #[test]
    fn parameter_text_cannot_be_reinterpreted_as_a_template_placeholder() {
        let mut text = json!("FIRST SECOND");
        substitute(
            &mut text,
            &[
                ("FIRST".into(), "SECOND".into()),
                ("SECOND".into(), "value".into()),
            ],
        );
        assert_eq!(text, json!("SECOND value"));
    }
    #[test]
    fn refuses_reserved_or_mistyped_sample_fields() {
        let mut parameters = contract()["template_parameters"]["crud.foundation"].clone();
        parameters["fields"][0]["name"] = json!("id");
        assert!(instantiate("crud.foundation", "1.0.0", &parameters).is_err());
        let mut parameters = contract()["template_parameters"]["crud.foundation"].clone();
        parameters["sample_update"]["native_field"] = json!(42);
        assert!(instantiate("crud.foundation", "1.0.0", &parameters).is_err());
    }
}
