use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use serde_path_to_error::{Path, Segment};

use super::ir_tools::typed_ir_schema_hint;

fn json_pointer(path: &Path) -> String {
    let mut pointer = String::new();
    for segment in path {
        pointer.push('/');
        match segment {
            Segment::Seq { index } => pointer.push_str(&index.to_string()),
            Segment::Map { key } => {
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
            }
            Segment::Enum { variant } => {
                pointer.push_str(&variant.replace('~', "~0").replace('/', "~1"));
            }
            Segment::Unknown => pointer.push('?'),
        }
    }
    if pointer.is_empty() {
        "/".to_string()
    } else {
        pointer
    }
}

fn value_at_path<'a>(root: &'a Value, path: &Path) -> Option<&'a Value> {
    let mut value = root;
    for segment in path {
        value = match segment {
            Segment::Seq { index } => value.as_array()?.get(*index)?,
            Segment::Map { key } => value.as_object()?.get(key)?,
            // Externally-tagged enums can contribute a path segment; typed Flow IR uses internal
            // tags, so keeping the current object is the most accurate fail-safe fallback.
            Segment::Enum { .. } | Segment::Unknown => value,
        };
    }
    Some(value)
}

fn json_kind(value: Option<&Value>) -> &'static str {
    match value {
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => "integer",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        None => "missing_or_unknown",
    }
}

fn expected_from_error(error: &str) -> String {
    error
        .rsplit_once("expected ")
        .map(|(_, expected)| expected)
        .unwrap_or("a value matching the advertised typed-IR schema")
        .split(" at line ")
        .next()
        .unwrap_or("a value matching the advertised typed-IR schema")
        .trim()
        .to_string()
}

/// Deserialize a typed-IR tool call with one provider-independent repair envelope. The input is
/// already a JSON value, so this never retains raw prompts or generated source beyond the call.
pub fn parse_typed_ir_arguments<T: DeserializeOwned>(
    arguments: Value,
    code: &str,
    context: &str,
) -> Result<T, String> {
    let encoded = serde_json::to_vec(&arguments).map_err(|error| {
        let canonical_example = typed_ir_schema_hint();
        json!({
            "status": "validation_errors",
            "code": code,
            "path": "/",
            "expected": "serializable JSON matching the advertised typed-IR schema",
            "actual_kind": json_kind(Some(&arguments)),
            "canonical_example": canonical_example,
            "message": format!("Failed to encode {context}: {error}")
        })
        .to_string()
    })?;
    let mut deserializer = serde_json::Deserializer::from_slice(&encoded);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = error.path().clone();
        let detail = error.inner().to_string();
        let canonical_example = typed_ir_schema_hint();
        json!({
            "status": "validation_errors",
            "code": code,
            "path": json_pointer(&path),
            "expected": expected_from_error(&detail),
            "actual_kind": json_kind(value_at_path(&arguments, &path)),
            "canonical_example": canonical_example,
            "message": format!("Failed to parse {context}: {detail}")
        })
        .to_string()
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct NestedArgs {
        items: Vec<Flag>,
    }

    #[derive(Debug, Deserialize)]
    struct Flag {
        enabled: bool,
    }

    #[test]
    fn parse_errors_include_path_kind_expectation_and_canonical_example() {
        let error = parse_typed_ir_arguments::<NestedArgs>(
            json!({ "items": [{ "enabled": "true" }] }),
            "IR_TEST_INVALID",
            "test arguments",
        )
        .expect_err("string is not a JSON boolean");
        let payload: Value = serde_json::from_str(&error).unwrap();
        assert_eq!(payload["status"], "validation_errors");
        assert_eq!(payload["code"], "IR_TEST_INVALID");
        assert_eq!(payload["path"], "/items/0/enabled");
        assert_eq!(payload["actual_kind"], "string");
        assert!(payload["expected"].as_str().unwrap().contains("boolean"));
        assert_eq!(
            payload["canonical_example"]["type_object"]["data_type"],
            "string"
        );
        assert_eq!(
            payload["canonical_example"]["function_cache"]["namespace"],
            "global"
        );
        assert_eq!(
            payload["canonical_example"]["function_cache"]["ttl_seconds"],
            300
        );
    }
}
