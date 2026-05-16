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

        if is_outdated_bit_schema(schema_value) {
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

fn is_outdated_bit_schema(schema: &str) -> bool {
    let Ok(schema) = serde_json::from_str::<Value>(schema) else {
        return false;
    };

    if !has_bit_schema_shape(&schema) {
        return false;
    }

    matches!(
        bit_types_enum_contains_required_variants(&schema),
        Some(false)
    )
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

fn bit_types_enum_contains_required_variants(schema: &Value) -> Option<bool> {
    match schema {
        Value::Object(object) => {
            if let Some(Value::Array(values)) = object.get("enum") {
                let enum_values: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
                if enum_values.contains(&"Llm")
                    && enum_values.contains(&"Vlm")
                    && enum_values.contains(&"Embedding")
                    && enum_values.contains(&"Other")
                {
                    return Some(enum_values.contains(&"Tts") && enum_values.contains(&"Stt"));
                }
            }

            object
                .values()
                .find_map(bit_types_enum_contains_required_variants)
        }
        Value::Array(values) => values
            .iter()
            .find_map(bit_types_enum_contains_required_variants),
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
                            .map_or(true, |variant| !variants.contains(&variant))
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
        assert_eq!(
            Some(migrated.as_str()),
            cleanup.bit_schema.as_ref().map(String::as_str)
        );
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
        assert_eq!(
            Some(migrated.as_str()),
            cleanup.bit_schema.as_ref().map(String::as_str)
        );
        assert!(migrated.contains("\"Stt\""));
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

        assert_eq!(
            variable.schema.as_deref(),
            cleanup.bit_schema.as_ref().map(String::as_str)
        );
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
