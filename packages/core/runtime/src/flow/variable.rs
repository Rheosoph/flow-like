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
    pub fn validate_value(&self, value: &Value) -> flow_like_types::Result<()> {
        validate_typed_value(
            &self.data_type,
            &self.value_type,
            self.schema.as_deref(),
            value,
        )
    }

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
    Geometry,
}

/// Resolve the frozen marker for a Geometry declaration. `None` means any shape.
pub fn geometry_kind_from_schema(
    schema: Option<&str>,
) -> flow_like_types::Result<Option<flow_like_types::geometry::GeometryKind>> {
    let Some(schema) = schema else {
        return Ok(None);
    };
    flow_like_types::geometry::kind_from_schema(schema)?
        .map(Some)
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Geometry schema must be a frozen flow:geometry subtype marker"
            )
        })
}

/// Validate geometry values without changing the existing behavior of other types.
pub fn validate_typed_value(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
    value: &Value,
) -> flow_like_types::Result<()> {
    if *data_type != VariableType::Geometry {
        return Ok(());
    }
    let kind = geometry_kind_from_schema(schema)?;
    match value_type {
        ValueType::Normal => flow_like_types::geometry::validate_geometry(value, kind)?,
        ValueType::Array | ValueType::HashSet => {
            let values = value.as_array().ok_or_else(|| {
                flow_like_types::anyhow!("Geometry Array/HashSet requires an array")
            })?;
            for (index, value) in values.iter().enumerate() {
                flow_like_types::geometry::validate_geometry(value, kind).map_err(|error| {
                    flow_like_types::anyhow!("Geometry member [{index}]: {error}")
                })?;
            }
        }
        ValueType::HashMap => {
            let values = value
                .as_object()
                .ok_or_else(|| flow_like_types::anyhow!("Geometry HashMap requires an object"))?;
            for (key, value) in values {
                flow_like_types::geometry::validate_geometry(value, kind).map_err(|error| {
                    flow_like_types::anyhow!("Geometry member [{key:?}]: {error}")
                })?;
            }
        }
    }
    Ok(())
}

/// An absent default or JSON null leaves the declaration unset. Supplied geometry
/// values, including every container member, must satisfy the geometry contract.
pub fn validate_typed_default(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
    default: Option<&[u8]>,
) -> flow_like_types::Result<()> {
    if *data_type != VariableType::Geometry {
        return Ok(());
    }
    geometry_kind_from_schema(schema)?;
    if let Some(default) = default {
        if default.len() > flow_like_types::geometry::MAX_GEOMETRY_BYTES {
            return Err(flow_like_types::anyhow!(
                "Geometry default exceeds the byte limit"
            ));
        }
        let value: Value = json::from_slice(default)?;
        if !value.is_null() {
            validate_typed_value(data_type, value_type, schema, &value)?;
        }
    }
    Ok(())
}

/// The value an absent optional declaration resolves to. Every result is fixed so default
/// bytes hash identically across machines; `Null` only where no valid empty shape exists.
pub fn default_value_for_type(
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
) -> Value {
    match value_type {
        ValueType::Array | ValueType::HashSet => return json::json!([]),
        ValueType::HashMap => return json::json!({}),
        ValueType::Normal => {}
    }
    match data_type {
        VariableType::String | VariableType::PathBuf => json::json!(""),
        VariableType::Integer | VariableType::Byte => json::json!(0),
        VariableType::Float => json::json!(0.0),
        VariableType::Boolean => json::json!(false),
        VariableType::Date => json::json!("1970-01-01T00:00:00Z"),
        VariableType::Struct => json::json!({}),
        VariableType::Geometry => empty_geometry(schema),
        VariableType::Generic | VariableType::Execution => Value::Null,
    }
}

fn empty_geometry(schema: Option<&str>) -> Value {
    use flow_like_types::geometry::GeometryKind;
    match geometry_kind_from_schema(schema).ok().flatten() {
        Some(
            kind @ (GeometryKind::MultiPoint
            | GeometryKind::MultiLineString
            | GeometryKind::MultiPolygon),
        ) => json::json!({"type": kind.as_str(), "coordinates": []}),
        Some(GeometryKind::Point | GeometryKind::LineString | GeometryKind::Polygon) => Value::Null,
        Some(GeometryKind::GeometryCollection) | None => {
            json::json!({"type": "GeometryCollection", "geometries": []})
        }
    }
}

/// The stored default when it is a non-null value, otherwise the type default.
pub fn effective_default(
    stored: Option<&Value>,
    data_type: &VariableType,
    value_type: &ValueType,
    schema: Option<&str>,
) -> Value {
    match stored {
        Some(value) if !value.is_null() => value.clone(),
        _ => default_value_for_type(data_type, value_type, schema),
    }
}

impl crate::flow::board::Board {
    /// Check every declared Geometry default before saving or compiling a board.
    pub fn validate_geometry_contracts(&self) -> flow_like_types::Result<()> {
        let check = |data_type: &VariableType,
                     value_type: &ValueType,
                     schema: Option<&str>,
                     default: Option<&[u8]>| {
            if *data_type != VariableType::Geometry {
                return Ok(());
            }
            let schema = schema
                .map(|schema| super::pin::resolve_schema(schema, &self.refs))
                .transpose()?;
            validate_typed_default(data_type, value_type, schema, default)
        };
        for variable in self.variables.values().chain(
            self.layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        ) {
            check(
                &variable.data_type,
                &variable.value_type,
                variable.schema.as_deref(),
                variable.default_value.as_deref(),
            )
            .map_err(|error| flow_like_types::anyhow!("Variable '{}': {error}", variable.name))?;
        }
        let pins = self
            .nodes
            .values()
            .flat_map(|node| {
                node.pins
                    .values()
                    .map(|pin| (node.friendly_name.as_str(), pin))
            })
            .chain(
                self.layers
                    .values()
                    .flat_map(|layer| layer.pins.values().map(|pin| (layer.name.as_str(), pin))),
            )
            .chain(
                self.layers
                    .values()
                    .flat_map(|layer| layer.nodes.values())
                    .flat_map(|node| {
                        node.pins
                            .values()
                            .map(|pin| (node.friendly_name.as_str(), pin))
                    }),
            );
        for (owner, pin) in pins {
            check(
                &pin.data_type,
                &pin.value_type,
                pin.schema.as_deref(),
                pin.default_value.as_deref(),
            )
            .map_err(|error| {
                flow_like_types::anyhow!("Pin '{}' on '{}': {error}", pin.friendly_name, owner)
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, tokio};

    #[test]
    fn geometry_containers_defaults_and_wire_codes_are_checked() {
        use super::*;
        use flow_like_types::geometry::{GeometryKind, marker};
        let point = json::json!({"type":"Point","coordinates":[13.405,52.52]});
        for (container, value) in [
            (ValueType::Normal, point.clone()),
            (ValueType::Array, json::json!([point])),
            (ValueType::HashSet, json::json!([point])),
            (ValueType::HashMap, json::json!({"berlin":point})),
        ] {
            validate_typed_value(
                &VariableType::Geometry,
                &container,
                Some(marker(GeometryKind::Point)),
                &value,
            )
            .unwrap();
            assert!(
                validate_typed_value(
                    &VariableType::Geometry,
                    &container,
                    Some(marker(GeometryKind::Polygon)),
                    &value
                )
                .is_err()
            );
            assert!(
                validate_typed_value(&VariableType::Geometry, &container, None, &Value::Null)
                    .is_err()
            );
        }
        validate_typed_default(&VariableType::Geometry, &ValueType::Normal, None, None).unwrap();
        assert!(geometry_kind_from_schema(Some(r#"{"type":"object"}"#)).is_err());
        assert_eq!(VariableType::Byte.to_proto(), 9);
        assert_eq!(VariableType::Geometry.to_proto(), 10);
        assert_eq!(
            VariableType::try_from_proto(10).unwrap(),
            VariableType::Geometry
        );
        assert!(VariableType::try_from_proto(11).is_err());
        assert!(VariableType::try_from_proto(-1).is_err());
    }

    #[test]
    fn geometry_defaults_allow_unset_declarations_but_reject_invalid_values() {
        use super::*;
        use flow_like_types::geometry::{GeometryKind, MAX_GEOMETRY_BYTES, marker};

        for schema in std::iter::once(None)
            .chain(GeometryKind::ALL.into_iter().map(|kind| Some(marker(kind))))
        {
            for container in [
                ValueType::Normal,
                ValueType::Array,
                ValueType::HashSet,
                ValueType::HashMap,
            ] {
                for default in [
                    None,
                    Some(b"null".as_slice()),
                    Some(b" \nnull\t".as_slice()),
                ] {
                    validate_typed_default(&VariableType::Geometry, &container, schema, default)
                        .unwrap();
                }
                // Unset declarations do not make null a valid runtime geometry.
                assert!(
                    validate_typed_value(&VariableType::Geometry, &container, schema, &Value::Null)
                        .is_err()
                );
            }
        }

        for (container, value) in [
            (ValueType::Normal, json::json!("null")),
            (ValueType::Normal, json::json!({})),
            (
                ValueType::Normal,
                json::json!({"type":"Point","coordinates":[181,0]}),
            ),
            (
                ValueType::Normal,
                json::json!({"type":"GeometryCollection","geometries":[null]}),
            ),
            (ValueType::Array, json::json!([null])),
            (ValueType::HashSet, json::json!([null])),
            (ValueType::HashMap, json::json!({"place":null})),
        ] {
            assert!(
                validate_typed_default(
                    &VariableType::Geometry,
                    &container,
                    None,
                    Some(&json::to_vec(&value).unwrap()),
                )
                .is_err(),
                "{container:?}: {value}"
            );
        }

        for schema in [Some("unresolved-ref"), Some(r#"{"type":"object"}"#)] {
            for default in [None, Some(b"null".as_slice())] {
                assert!(
                    validate_typed_default(
                        &VariableType::Geometry,
                        &ValueType::Normal,
                        schema,
                        default,
                    )
                    .is_err()
                );
            }
        }
        let oversized_null = format!("null{}", " ".repeat(MAX_GEOMETRY_BYTES));
        for default in [b"".as_slice(), b"nul", oversized_null.as_bytes()] {
            assert!(
                validate_typed_default(
                    &VariableType::Geometry,
                    &ValueType::Normal,
                    None,
                    Some(default),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn type_defaults_cover_every_cell_and_geometry_defaults_validate() {
        use super::*;
        use flow_like_types::geometry::{GeometryKind, marker};
        let collection = json::json!({"type": "GeometryCollection", "geometries": []});
        let containers = [
            (ValueType::Array, json::json!([])),
            (ValueType::HashSet, json::json!([])),
            (ValueType::HashMap, json::json!({})),
        ];
        let scalars = [
            (VariableType::String, json::json!("")),
            (VariableType::PathBuf, json::json!("")),
            (VariableType::Integer, json::json!(0)),
            (VariableType::Byte, json::json!(0)),
            (VariableType::Float, json::json!(0.0)),
            (VariableType::Boolean, json::json!(false)),
            (VariableType::Date, json::json!("1970-01-01T00:00:00Z")),
            (VariableType::Struct, json::json!({})),
            (VariableType::Generic, Value::Null),
            (VariableType::Execution, Value::Null),
            (VariableType::Geometry, collection.clone()),
        ];
        for (data_type, expected) in &scalars {
            assert_eq!(
                default_value_for_type(data_type, &ValueType::Normal, None),
                *expected,
                "{data_type:?}"
            );
            for (container, expected) in &containers {
                assert_eq!(
                    default_value_for_type(data_type, container, None),
                    *expected,
                    "{data_type:?} {container:?}"
                );
            }
        }
        assert!(default_value_for_type(&VariableType::Float, &ValueType::Normal, None).is_f64());

        let empty = |kind: GeometryKind| json::json!({"type": kind.as_str(), "coordinates": []});
        for (kind, expected) in [
            (GeometryKind::Point, Value::Null),
            (GeometryKind::LineString, Value::Null),
            (GeometryKind::Polygon, Value::Null),
            (GeometryKind::MultiPoint, empty(GeometryKind::MultiPoint)),
            (
                GeometryKind::MultiLineString,
                empty(GeometryKind::MultiLineString),
            ),
            (
                GeometryKind::MultiPolygon,
                empty(GeometryKind::MultiPolygon),
            ),
            (GeometryKind::GeometryCollection, collection.clone()),
        ] {
            let schema = Some(marker(kind));
            let value = default_value_for_type(&VariableType::Geometry, &ValueType::Normal, schema);
            assert_eq!(value, expected, "{kind:?}");
            if !value.is_null() {
                validate_typed_value(&VariableType::Geometry, &ValueType::Normal, schema, &value)
                    .unwrap();
            }
            for (container, _) in &containers {
                let value = default_value_for_type(&VariableType::Geometry, container, schema);
                validate_typed_value(&VariableType::Geometry, container, schema, &value).unwrap();
            }
        }
        for schema in [None, Some("unresolved-ref"), Some(r#"{"type":"object"}"#)] {
            let value = default_value_for_type(&VariableType::Geometry, &ValueType::Normal, schema);
            assert_eq!(value, collection, "{schema:?}");
            validate_typed_value(&VariableType::Geometry, &ValueType::Normal, None, &value)
                .unwrap();
        }
    }

    #[test]
    fn effective_default_prefers_a_non_null_stored_value() {
        use super::*;
        assert_eq!(
            effective_default(
                Some(&json::json!(7)),
                &VariableType::Integer,
                &ValueType::Normal,
                None
            ),
            json::json!(7)
        );
        assert_eq!(
            effective_default(
                Some(&json::json!(false)),
                &VariableType::Boolean,
                &ValueType::Normal,
                None
            ),
            json::json!(false)
        );
        assert_eq!(
            effective_default(
                Some(&Value::Null),
                &VariableType::Integer,
                &ValueType::Normal,
                None
            ),
            json::json!(0)
        );
        assert_eq!(
            effective_default(None, &VariableType::String, &ValueType::Array, None),
            json::json!([])
        );
        assert_eq!(
            effective_default(None, &VariableType::Generic, &ValueType::Normal, None),
            Value::Null
        );
    }

    #[test]
    fn geometry_schemas_resolve_collections_when_embedded_in_containers() {
        use flow_like_types::geometry::{GeometryKind, geometry_json_schema, geometry_tool_schema};
        use flow_like_types::json;
        let collection = json::json!({"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[13.405,52.52]}]});
        for kind in [None, Some(GeometryKind::GeometryCollection)] {
            let schema = json::json!({"type":"object","properties":{"places":{"type":"array","items":geometry_json_schema(kind)}},"required":["places"]});
            let validator = jsonschema::validator_for(&schema).unwrap();
            assert!(validator.is_valid(&json::json!({"places":[collection]})));
            assert!(!validator.is_valid(&json::json!({"places":[{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[0,91]}]}]})));
        }
        for kind in std::iter::once(None).chain(GeometryKind::ALL.into_iter().map(Some)) {
            let projection = geometry_tool_schema(kind);
            assert!(projection.get("$ref").is_none());
            assert!(projection["properties"]["type"]["enum"].is_array());
        }
    }

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
