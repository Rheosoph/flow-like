use std::collections::HashMap;

use canonical_json::ser::to_string as canonical_json_string;
use flow_like_types::json::to_value;
use schemars::schema_for;
use serde_json::Value;

use crate::{
    bit::Bit,
    flow::{
        board::{
            Board,
            cleanup::{BoardCleanupLogic, PinLookup},
        },
        pin::Pin,
        variable::{Variable, VariableType},
    },
};

pub struct SyncKnownSchemasCleanup {
    refs: HashMap<String, String>,
    bit_schema: Option<String>,
}

impl SyncKnownSchemasCleanup {
    fn current_bit_schema() -> Option<String> {
        let schema = schema_for!(Bit);
        to_value(&schema)
            .ok()
            .and_then(|value| canonical_json_string(&value).ok())
    }

    fn resolve_schema<'a>(&'a self, schema: &'a str) -> Option<&'a str> {
        if looks_like_json(schema) {
            return Some(schema);
        }

        self.refs.get(schema).map(String::as_str)
    }

    fn sync_schema(&self, schema: &mut Option<String>) {
        let Some(current_bit_schema) = &self.bit_schema else {
            return;
        };

        let Some(schema_ref) = schema.as_deref() else {
            return;
        };

        let Some(schema_value) = self.resolve_schema(schema_ref) else {
            return;
        };

        if is_outdated_bit_schema(schema_value, current_bit_schema) {
            *schema = Some(current_bit_schema.clone());
        }
    }
}

impl BoardCleanupLogic for SyncKnownSchemasCleanup {
    fn init(board: &mut Board) -> Self
    where
        Self: Sized,
    {
        Self {
            refs: board.refs.clone(),
            bit_schema: Self::current_bit_schema(),
        }
    }

    fn main_pin_iteration(&mut self, pin: &mut Pin, _pin_lookup: &PinLookup) {
        if pin.data_type == VariableType::Struct {
            self.sync_schema(&mut pin.schema);
        }
    }

    fn main_variable_iteration(&mut self, variable: &mut Variable, _pin_lookup: &PinLookup) {
        if variable.data_type == VariableType::Struct {
            self.sync_schema(&mut variable.schema);
        }
    }
}

fn looks_like_json(value: &str) -> bool {
    matches!(
        value.trim_start().as_bytes().first(),
        Some(b'{') | Some(b'[')
    )
}

/// Whether `schema` is a copy of `Bit` taken before `BitTypes` gained a variant the current
/// schema declares. Variables and Function parameters keep the copy they were typed with, and a
/// stale one refuses every current `Bit` assigned to it.
fn is_outdated_bit_schema(schema: &str, current_bit_schema: &str) -> bool {
    let Ok(schema) = serde_json::from_str::<Value>(schema) else {
        return false;
    };

    if !has_bit_schema_shape(&schema) {
        return false;
    }

    let Some(stored) = bit_types_variants(&schema) else {
        return false;
    };
    let Ok(current) = serde_json::from_str::<Value>(current_bit_schema) else {
        return false;
    };
    bit_types_variants(&current)
        .is_some_and(|current| current.iter().any(|variant| !stored.contains(variant)))
}

fn has_bit_schema_shape(schema: &Value) -> bool {
    if schema.get("title").and_then(Value::as_str) != Some("Bit") {
        return false;
    }

    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return false;
    };

    properties.contains_key("id")
        && properties.contains_key("type")
        && properties.contains_key("parameters")
        && properties.contains_key("dependencies")
        && properties.contains_key("dependency_tree_hash")
}

/// The `BitTypes` variants a Bit schema declares: the enum listing `Llm`, `Vlm`, `Embedding` and
/// `Other`.
fn bit_types_variants(schema: &Value) -> Option<Vec<&str>> {
    match schema {
        Value::Object(object) => {
            if let Some(Value::Array(values)) = object.get("enum") {
                let variants: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
                if ["Llm", "Vlm", "Embedding", "Other"]
                    .iter()
                    .all(|variant| variants.contains(variant))
                {
                    return Some(variants);
                }
            }

            object.values().find_map(bit_types_variants)
        }
        Value::Array(values) => values.iter().find_map(bit_types_variants),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::pin::ValueType;

    fn old_bit_schema() -> String {
        let mut schema = serde_json::from_str::<Value>(
            &SyncKnownSchemasCleanup::current_bit_schema().expect("Bit schema should serialize"),
        )
        .expect("Bit schema should parse");
        remove_bit_type_enum_values(&mut schema, &["Tts"]);
        canonical_json_string(&schema).expect("Old Bit schema should serialize")
    }

    fn bit_schema_without_stt() -> String {
        let mut schema = serde_json::from_str::<Value>(
            &SyncKnownSchemasCleanup::current_bit_schema().expect("Bit schema should serialize"),
        )
        .expect("Bit schema should parse");
        remove_bit_type_enum_values(&mut schema, &["Stt"]);
        canonical_json_string(&schema).expect("Old Bit schema should serialize")
    }

    fn remove_bit_type_enum_values(value: &mut Value, variants: &[&str]) {
        match value {
            Value::Object(object) => {
                if let Some(Value::Array(values)) = object.get_mut("enum") {
                    values.retain(|value| {
                        value
                            .as_str()
                            .is_none_or(|variant| !variants.contains(&variant))
                    });
                }

                for child in object.values_mut() {
                    remove_bit_type_enum_values(child, variants);
                }
            }
            Value::Array(values) => {
                for child in values {
                    remove_bit_type_enum_values(child, variants);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn migrates_raw_old_bit_variable_schema() {
        let mut cleanup = SyncKnownSchemasCleanup {
            refs: HashMap::new(),
            bit_schema: SyncKnownSchemasCleanup::current_bit_schema(),
        };
        let mut variable = Variable::new("model", VariableType::Struct, ValueType::Normal);
        variable.schema = Some(old_bit_schema());

        cleanup.main_variable_iteration(&mut variable, &HashMap::new());

        let migrated = variable.schema.expect("schema should remain set");
        assert_eq!(Some(migrated.as_str()), cleanup.bit_schema.as_deref());
        assert!(migrated.contains("\"Tts\""));
        assert!(migrated.contains("\"Stt\""));
    }

    #[test]
    fn migrates_bit_variable_schema_missing_stt() {
        let mut cleanup = SyncKnownSchemasCleanup {
            refs: HashMap::new(),
            bit_schema: SyncKnownSchemasCleanup::current_bit_schema(),
        };
        let mut variable = Variable::new("model", VariableType::Struct, ValueType::Normal);
        variable.schema = Some(bit_schema_without_stt());

        cleanup.main_variable_iteration(&mut variable, &HashMap::new());

        let migrated = variable.schema.expect("schema should remain set");
        assert_eq!(Some(migrated.as_str()), cleanup.bit_schema.as_deref());
        assert!(migrated.contains("\"Stt\""));
    }

    /// `BitTypes` gained `SystemOne` and `Forecasting` after `Tts`/`Stt`; a copy taken in between
    /// must be refreshed too, on a variable and on a Function parameter alike.
    #[test]
    fn migrates_bit_schemas_missing_any_current_variant() {
        let mut cleanup = SyncKnownSchemasCleanup {
            refs: HashMap::new(),
            bit_schema: SyncKnownSchemasCleanup::current_bit_schema(),
        };
        let current = cleanup
            .bit_schema
            .clone()
            .expect("Bit schema should serialize");
        let mut stale = serde_json::from_str::<Value>(&current).expect("Bit schema should parse");
        remove_bit_type_enum_values(&mut stale, &["SystemOne", "Forecasting"]);
        let stale = canonical_json_string(&stale).expect("stale Bit schema should serialize");

        let mut variable = Variable::new("model", VariableType::Struct, ValueType::Normal);
        variable.schema = Some(stale.clone());
        cleanup.main_variable_iteration(&mut variable, &HashMap::new());
        assert_eq!(variable.schema.as_deref(), Some(current.as_str()));

        let mut function = crate::flow::node::Node::new("function", "Function", "", "");
        let mut parameter = function
            .add_input_pin("model", "Model", "", VariableType::Struct)
            .clone();
        parameter.schema = Some(stale);
        cleanup.main_pin_iteration(&mut parameter, &HashMap::new());
        assert_eq!(parameter.schema.as_deref(), Some(current.as_str()));

        let mut up_to_date = Variable::new("model", VariableType::Struct, ValueType::Normal);
        up_to_date.schema = Some(current.clone());
        cleanup.main_variable_iteration(&mut up_to_date, &HashMap::new());
        assert_eq!(up_to_date.schema.as_deref(), Some(current.as_str()));
    }

    #[test]
    fn migrates_ref_old_bit_variable_schema() {
        let old_schema = old_bit_schema();
        let mut cleanup = SyncKnownSchemasCleanup {
            refs: HashMap::from([("old_bit_schema".to_string(), old_schema)]),
            bit_schema: SyncKnownSchemasCleanup::current_bit_schema(),
        };
        let mut variable = Variable::new("model", VariableType::Struct, ValueType::Normal);
        variable.schema = Some("old_bit_schema".to_string());

        cleanup.main_variable_iteration(&mut variable, &HashMap::new());

        assert_eq!(variable.schema.as_deref(), cleanup.bit_schema.as_deref());
    }

    #[test]
    fn leaves_unrelated_struct_schema_unchanged() {
        let mut cleanup = SyncKnownSchemasCleanup {
            refs: HashMap::new(),
            bit_schema: SyncKnownSchemasCleanup::current_bit_schema(),
        };
        let schema = r#"{"type":"object","properties":{"id":{"type":"string"}}}"#.to_string();
        let mut variable = Variable::new("other", VariableType::Struct, ValueType::Normal);
        variable.schema = Some(schema.clone());

        cleanup.main_variable_iteration(&mut variable, &HashMap::new());

        assert_eq!(variable.schema.as_deref(), Some(schema.as_str()));
    }
}
