use super::variable::VariableType;
use canonical_json::ser::to_string;
use flow_like_types::{Value, json::to_value, sync::Mutex};
use highway::{HighwayHash, HighwayHasher};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub enum PinType {
    Input,
    Output,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct PinOptions {
    pub sensitive: Option<bool>,
    pub valid_values: Option<Vec<String>>,
    pub range: Option<(f64, f64)>,
    pub step: Option<f64>,
    pub enforce_schema: Option<bool>,
    pub enforce_generic_value_type: Option<bool>,
}

impl Default for PinOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl PinOptions {
    pub fn new() -> Self {
        PinOptions {
            sensitive: None,
            valid_values: None,
            range: None,
            step: None,
            enforce_schema: None,
            enforce_generic_value_type: None,
        }
    }

    pub fn set_valid_values(&mut self, valid_values: Vec<String>) -> &mut Self {
        self.valid_values = Some(valid_values);
        self
    }

    pub fn set_range(&mut self, range: (f64, f64)) -> &mut Self {
        self.range = Some(range);
        self
    }

    pub fn set_sensitive(&mut self, sensitive: bool) -> &mut Self {
        self.sensitive = Some(sensitive);
        self
    }

    pub fn set_step(&mut self, step: f64) -> &mut Self {
        self.step = Some(step);
        self
    }

    pub fn set_enforce_schema(&mut self, enforce_schema: bool) -> &mut Self {
        self.enforce_schema = Some(enforce_schema);
        self
    }

    pub fn set_enforce_generic_value_type(
        &mut self,
        enforce_generic_value_type: bool,
    ) -> &mut Self {
        self.enforce_generic_value_type = Some(enforce_generic_value_type);
        self
    }

    pub fn build(&self) -> Self {
        self.clone()
    }

    pub fn hash_into(&self, hasher: &mut HighwayHasher) {
        if let Some(sensitive) = &self.sensitive {
            hasher.append(sensitive.to_string().as_bytes());
        }
        if let Some(valid_values) = &self.valid_values {
            for value in valid_values {
                hasher.append(value.as_bytes());
            }
        }
        if let Some((min, max)) = &self.range {
            hasher.append(&min.to_le_bytes());
            hasher.append(&max.to_le_bytes());
        }
        if let Some(step) = &self.step {
            hasher.append(&step.to_le_bytes());
        }
        if let Some(enforce_schema) = &self.enforce_schema {
            hasher.append(enforce_schema.to_string().as_bytes());
        }
        if let Some(enforce_generic_value_type) = &self.enforce_generic_value_type {
            hasher.append(enforce_generic_value_type.to_string().as_bytes());
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Pin {
    pub id: String,
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub pin_type: PinType,
    pub data_type: VariableType,
    pub schema: Option<String>,
    pub value_type: ValueType,
    pub depends_on: BTreeSet<String>,
    pub connected_to: BTreeSet<String>,
    pub default_value: Option<Vec<u8>>,
    pub index: u16,
    pub options: Option<PinOptions>,

    // This will be set on execution, for execution it will be "Null"
    #[serde(skip)]
    pub value: Option<Arc<Mutex<Value>>>,
}

/// Schema for a Struct pin whose fields are supplied by the user or a remote
/// service. See [`Pin::set_open_schema`].
pub const OPEN_OBJECT_SCHEMA: &str = r#"{"type":"object","additionalProperties":true}"#;

/// Whether `schema` is the open-object marker rather than a real shape.
///
/// [`OPEN_OBJECT_SCHEMA`] declares that a pin's fields are open, so it can never contradict a
/// concrete schema. Every site that compares two pin schemas must treat it as an absent schema,
/// not as a contract the peer has to equal. Mirrored in TypeScript by `isOpenObjectSchema` in
/// `packages/ui/lib/flow-board-utils.tsx`.
pub fn is_open_object_schema(schema: &str) -> bool {
    if !schema.contains("additionalProperties") {
        return false;
    }
    let Ok(Value::Object(fields)) = flow_like_types::json::from_str::<Value>(schema) else {
        return false;
    };
    fields.len() == 2
        && fields.get("type").and_then(Value::as_str) == Some("object")
        && fields.get("additionalProperties").and_then(Value::as_bool) == Some(true)
}

/// Whether two declared pin schemas can coexist on a connection.
///
/// Only two concrete schemas can contradict one another: an absent schema declares nothing, and an
/// open-object schema declares that the shape is open. See [`is_open_object_schema`].
pub fn schemas_are_compatible(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left == right || is_open_object_schema(left) || is_open_object_schema(right)
        }
        _ => true,
    }
}

impl Pin {
    /// Whether the pin's literal is a secret (API keys, passwords) that must never leave the
    /// server in a board response and is write-only from clients.
    pub fn is_sensitive(&self) -> bool {
        self.options
            .as_ref()
            .and_then(|options| options.sensitive)
            .unwrap_or(false)
    }

    /// Write-only semantics for sensitive literals: a client that received a board never saw the
    /// value, so an incoming `None` means "unchanged", not "clear". Clearing is an explicit empty
    /// value. Call this on an incoming pin with the pin the board currently holds.
    pub fn keep_sensitive_value_from(&mut self, existing: Option<&Pin>) {
        if !self.is_sensitive() || self.default_value.is_some() {
            return;
        }
        if let Some(existing) = existing
            && existing.is_sensitive()
        {
            self.default_value = existing.default_value.clone();
        }
    }

    pub fn set_default_value(&mut self, default_value: Option<Value>) -> &mut Self {
        self.default_value = default_value.map(|v| flow_like_types::json::to_vec(&v).unwrap());
        self
    }

    pub fn set_value_type(&mut self, value_type: ValueType) -> &mut Self {
        self.value_type = value_type;
        self
    }

    pub fn set_data_type(&mut self, data_type: VariableType) -> &mut Self {
        self.data_type = data_type;
        self
    }

    /// Declares a Struct pin whose fields are supplied by the user or the remote
    /// service, so no fixed shape exists to describe — a config map, a database
    /// row, a decoded payload. This is a statement that the shape is open, not a
    /// placeholder: a pin that *does* have a known shape should use
    /// [`Pin::set_schema`] instead.
    pub fn set_open_schema(&mut self) -> &mut Self {
        self.schema = Some(OPEN_OBJECT_SCHEMA.to_string());
        self
    }

    /// See [`is_open_object_schema`]: the pin declares an open shape, so it constrains nothing.
    pub fn has_open_schema(&self) -> bool {
        self.schema.as_deref().is_some_and(is_open_object_schema)
    }

    pub fn set_schema<T: Serialize + JsonSchema>(&mut self) -> &mut Self {
        let schema = schema_for!(T);
        let schema_str = to_value(&schema).ok().and_then(|v| to_string(&v).ok());
        self.schema = schema_str;
        self
    }

    pub fn reset_schema(&mut self) -> &mut Self {
        self.schema = None;
        self
    }

    pub fn set_options(&mut self, options: PinOptions) -> &mut Self {
        self.options = Some(options);
        self
    }

    pub fn hash_into(&self, hasher: &mut HighwayHasher) {
        hasher.append(self.id.as_bytes());
        hasher.append(self.name.as_bytes());
        hasher.append(self.friendly_name.as_bytes());
        hasher.append(self.description.as_bytes());
        hasher.append(&[self.value_type.clone() as u8]);
        hasher.append(&self.index.to_le_bytes());
        hasher.append(&[self.pin_type.clone() as u8]);
        hasher.append(&[self.data_type.clone() as u8]);
        if let Some(schema) = &self.schema {
            hasher.append(schema.as_bytes());
        }

        if let Some(options) = &self.options {
            options.hash_into(hasher);
        }

        for connected in &self.connected_to {
            hasher.append(connected.as_bytes());
        }

        if let Some(default_value) = &self.default_value {
            hasher.append(default_value);
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Array,
    Normal,
    HashMap,
    HashSet,
}

impl Pin {}

#[cfg(test)]
mod tests {

    use flow_like_types::sync::Mutex;
    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, Value, tokio};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    #[test]
    fn the_open_marker_is_recognized_however_it_is_formatted() {
        for schema in [
            super::OPEN_OBJECT_SCHEMA,
            r#"{ "additionalProperties" : true , "type" : "object" }"#,
            "{\n  \"type\": \"object\",\n  \"additionalProperties\": true\n}",
        ] {
            assert!(
                super::is_open_object_schema(schema),
                "should be the open marker: {schema}"
            );
        }
    }

    #[test]
    fn real_schemas_are_never_mistaken_for_the_open_marker() {
        for schema in [
            r#"{"type":"object","properties":{"sub":{"type":"string"}}}"#,
            // Declares fields *and* allows extras — a real contract, not a wildcard.
            r#"{"type":"object","additionalProperties":true,"properties":{"x":{}}}"#,
            r#"{"type":"object","additionalProperties":false}"#,
            r#"{"type":"array","additionalProperties":true}"#,
            "not json",
            "",
        ] {
            assert!(
                !super::is_open_object_schema(schema),
                "should not be the open marker: {schema}"
            );
        }
    }

    #[test]
    fn only_two_concrete_schemas_can_contradict_each_other() {
        let real = r#"{"title":"UserExecutionContext"}"#;
        let other = r#"{"title":"Bit"}"#;

        assert!(super::schemas_are_compatible(Some(real), Some(real)));
        assert!(super::schemas_are_compatible(Some(real), None));
        assert!(super::schemas_are_compatible(None, None));
        assert!(super::schemas_are_compatible(
            Some(real),
            Some(super::OPEN_OBJECT_SCHEMA)
        ));
        assert!(super::schemas_are_compatible(
            Some(super::OPEN_OBJECT_SCHEMA),
            Some(real)
        ));
        assert!(!super::schemas_are_compatible(Some(real), Some(other)));
    }

    #[tokio::test]
    async fn serialize_pin() {
        let pin = super::Pin {
            id: "123".to_string(),
            name: "name".to_string(),
            friendly_name: "friendly_name".to_string(),
            description: "description".to_string(),
            pin_type: super::PinType::Input,
            data_type: super::VariableType::Execution,
            schema: None,
            value_type: super::ValueType::Normal,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value: None,
            index: 0,
            options: None,
            value: Some(Arc::new(Mutex::new(Value::Null))),
        };
        // let pin = super::SerializablePin::from(pin);

        let mut buf = Vec::new();
        pin.to_proto().encode(&mut buf).unwrap();
        let deser = super::Pin::from_proto(flow_like_types::proto::Pin::decode(&buf[..]).unwrap());

        assert_eq!(pin.id, deser.id);
    }
}
