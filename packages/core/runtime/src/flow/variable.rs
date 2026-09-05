use flow_like_types::{Value, create_id, json, sync::Mutex};
use highway::{HighwayHash, HighwayHasher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::pin::ValueType;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Variable {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub default_value: Option<Vec<u8>>,
    pub data_type: VariableType,
    pub value_type: ValueType,
    pub exposed: bool,
    pub secret: bool,
    pub editable: bool,
    pub hash: Option<u64>,
    pub schema: Option<String>,
    /// If true, this variable is configured per-user at runtime (stored locally, not in flow)
    #[serde(default)]
    pub runtime_configured: bool,

    #[serde(skip)]
    pub value: Arc<Mutex<Value>>,
}

impl PartialEq for Variable {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.category == other.category
            && self.description == other.description
            && self.default_value == other.default_value
            && self.data_type == other.data_type
            && self.value_type == other.value_type
            && self.exposed == other.exposed
            && self.secret == other.secret
            && self.editable == other.editable
            && self.schema == other.schema
            && self.runtime_configured == other.runtime_configured
        // Intentionally excluding self.value comparison
    }
}

impl Eq for Variable {}

impl Variable {
    pub fn new(name: &str, data_type: VariableType, value_type: ValueType) -> Self {
        Self {
            id: create_id(),
            name: name.to_string(),
            category: None,
            description: None,
            default_value: None,
            data_type,
            value_type,
            exposed: false,
            secret: false,
            editable: true,
            value: Arc::new(Mutex::new(Value::Null)),
            hash: None,
            schema: None,
            runtime_configured: false,
        }
    }

    pub fn duplicate(&self) -> Self {
        Self {
            id: create_id(),
            name: self.name.clone(),
            category: self.category.clone(),
            description: self.description.clone(),
            default_value: self.default_value.clone(),
            data_type: self.data_type.clone(),
            value_type: self.value_type.clone(),
            exposed: self.exposed,
            secret: self.secret,
            editable: self.editable,
            value: Arc::new(Mutex::new(Value::Null)),
            hash: None,
            schema: self.schema.clone(),
            runtime_configured: self.runtime_configured,
        }
    }

    pub fn set_editable(&mut self, editable: bool) -> &mut Self {
        self.editable = editable;
        self
    }

    pub fn set_exposed(&mut self, exposed: bool) -> &mut Self {
        self.exposed = exposed;
        self
    }

    pub fn set_secret(&mut self, secret: bool) -> &mut Self {
        self.secret = secret;
        self
    }

    pub fn set_runtime_configured(&mut self, runtime_configured: bool) -> &mut Self {
        self.runtime_configured = runtime_configured;
        self
    }

    pub fn set_category(&mut self, category: String) -> &mut Self {
        self.category = Some(category);
        self
    }

    pub fn set_description(&mut self, description: String) -> &mut Self {
        self.description = Some(description);
        self
    }

    pub fn set_default_value(&mut self, default_value: Value) -> &mut Self {
        self.default_value = Some(flow_like_types::json::to_vec(&default_value).unwrap());
        self
    }

    pub fn get_value(&self) -> Arc<Mutex<Value>> {
        self.value.clone()
    }

    pub fn hash(&mut self) {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543200,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.name.as_bytes());

        if let Some(category) = &self.category {
            hasher.append(category.as_bytes());
        }

        if let Some(description) = &self.description {
            hasher.append(description.as_bytes());
        }

        // We don´t leak secret values in the hash
        if !self.secret
            && let Some(default_value) = &self.default_value
        {
            hasher.append(default_value);
        }

        if let Some(schema) = &self.schema {
            hasher.append(schema.as_bytes());
        }

        hasher.append(format!("{:?}", self.data_type).as_bytes());
        hasher.append(format!("{:?}", self.value_type).as_bytes());
        hasher.append(&[self.exposed as u8]);
        hasher.append(&[self.secret as u8]);
        hasher.append(&[self.editable as u8]);
        hasher.append(&[self.runtime_configured as u8]);

        self.hash = Some(hasher.finalize64());
    }

    pub fn set_schema(&mut self, schema: Option<String>) -> &mut Self {
        self.schema = schema;
        self
    }

    /// Infer and set schema from example JSON or validate existing schema.
    /// If input looks like a JSON Schema, validates it. Otherwise infers schema from the JSON value.
    /// Returns the normalized schema string on success.
    pub fn infer_schema_from_json(&mut self, raw: &str) -> flow_like_types::Result<String> {
        let schema = infer_schema_from_json(raw)?;
        self.schema = Some(schema.clone());
        Ok(schema)
    }
}

/// Check if a JSON value looks like a JSON Schema
fn looks_like_schema(value: &Value) -> bool {
    const SCHEMA_KEYWORDS: &[&str] = &[
        "type",
        "properties",
        "items",
        "$schema",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "required",
        "additionalProperties",
        "patternProperties",
        "enum",
        "const",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
        "format",
        "definitions",
        "$defs",
    ];

    value
        .as_object()
        .is_some_and(|obj| SCHEMA_KEYWORDS.iter().any(|kw| obj.contains_key(*kw)))
}

/// Infer a JSON Schema from example JSON or validate an existing schema.
/// Returns the schema as a JSON string.
/// Only infers from objects/arrays - primitive values (numbers, strings, bools) are rejected
/// to avoid accidentally treating hash references as example data.
pub fn infer_schema_from_json(raw: &str) -> flow_like_types::Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(flow_like_types::anyhow!("Schema input cannot be empty"));
    }

    let user_json = json::from_str::<Value>(trimmed).map_err(|e| {
        flow_like_types::anyhow!(
            "Schema must be valid JSON (either a JSON Schema or an example JSON). Parse error: {e}"
        )
    })?;

    let is_schema = looks_like_schema(&user_json) && jsonschema::meta::is_valid(&user_json);
    let inferred = if is_schema {
        user_json
    } else {
        // Only infer schema from objects or arrays - primitive values (numbers, strings, bools, null)
        // should not be used for inference as they might be hash references or other non-example data
        if !user_json.is_object() && !user_json.is_array() {
            return Err(flow_like_types::anyhow!(
                "Schema must be a JSON Schema object or example JSON object/array, not a primitive value"
            ));
        }
        let schema = schemars::schema_for_value!(&user_json);
        let string = json::to_string_pretty(&schema)?;
        json::from_str(&string)?
    };

    json::to_string_pretty(&inferred)
        .map_err(|e| flow_like_types::anyhow!("Failed to serialize schema: {e}"))
}

#[derive(PartialEq, Eq, Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum VariableType {
    Execution,
    String,
    Integer,
    Float,
    Boolean,
    Date,
    PathBuf,
    Generic,
    Struct,
    Byte,
}

#[cfg(test)]
mod tests {
    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, tokio};

    #[tokio::test]
    async fn serialize_variable() {
        let variable = super::Variable::new(
            "name",
            super::VariableType::Execution,
            super::ValueType::Normal,
        );

        let mut buf = Vec::new();
        variable.to_proto().encode(&mut buf).unwrap();
        let deser = super::Variable::from_proto(
            flow_like_types::proto::Variable::decode(&buf[..]).unwrap(),
        );

        assert_eq!(variable.id, deser.id);
    }
}
