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
    pub optional: Option<bool>,
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
            optional: None,
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

    pub fn set_optional(&mut self, optional: bool) -> &mut Self {
        self.optional = Some(optional);
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
        if let Some(optional) = &self.optional {
            hasher.append(optional.to_string().as_bytes());
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
    let marker_kind = |schema: Option<&str>| {
        schema.and_then(|schema| {
            flow_like_types::geometry::kind_from_schema(schema)
                .ok()
                .flatten()
        })
    };
    let left_kind = marker_kind(left);
    let right_kind = marker_kind(right);
    if left_kind.is_some() || right_kind.is_some() {
        return (left.is_none() || left_kind.is_some())
            && (right.is_none() || right_kind.is_some())
            && flow_like_types::geometry::compatible(left_kind, right_kind);
    }
    match (left, right) {
        (Some(left), Some(right)) => {
            left == right || is_open_object_schema(left) || is_open_object_schema(right)
        }
        _ => true,
    }
}

/// Resolve compact schema references before interpreting a subtype marker.
pub fn resolve_schema<'a>(
    schema: &'a str,
    refs: &'a std::collections::HashMap<String, String>,
) -> flow_like_types::Result<&'a str> {
    let mut schema = schema;
    let mut seen = std::collections::HashSet::new();
    while let Some(resolved) = refs.get(schema) {
        if !seen.insert(schema) {
            return Err(flow_like_types::anyhow!("Cyclic schema reference"));
        }
        schema = resolved;
    }
    Ok(schema)
}

/// Geometry connections always enforce directional subtypes. Generic peers retain
/// their existing container policy and actual Geometry inputs validate at runtime.
pub fn geometry_pins_are_compatible(
    source: &Pin,
    target: &Pin,
    refs: &std::collections::HashMap<String, String>,
) -> flow_like_types::Result<bool> {
    if source.data_type != VariableType::Geometry && target.data_type != VariableType::Geometry {
        return Ok(true);
    }
    for pin in [source, target] {
        if pin.data_type == VariableType::Geometry {
            let schema = pin
                .schema
                .as_deref()
                .map(|schema| resolve_schema(schema, refs))
                .transpose()?;
            super::variable::geometry_kind_from_schema(schema)?;
        }
    }
    if source.data_type == VariableType::Generic || target.data_type == VariableType::Generic {
        let enforce = source
            .options
            .as_ref()
            .and_then(|options| options.enforce_generic_value_type)
            .unwrap_or(false)
            || target
                .options
                .as_ref()
                .and_then(|options| options.enforce_generic_value_type)
                .unwrap_or(false);
        return Ok(!enforce || source.value_type == target.value_type);
    }
    if source.data_type != target.data_type || source.value_type != target.value_type {
        return Ok(false);
    }
    let source_kind = super::variable::geometry_kind_from_schema(
        source
            .schema
            .as_deref()
            .map(|schema| resolve_schema(schema, refs))
            .transpose()?,
    )?;
    let target_kind = super::variable::geometry_kind_from_schema(
        target
            .schema
            .as_deref()
            .map(|schema| resolve_schema(schema, refs))
            .transpose()?,
    )?;
    Ok(flow_like_types::geometry::compatible(
        source_kind,
        target_kind,
    ))
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

    pub fn is_optional(&self) -> bool {
        self.options
            .as_ref()
            .and_then(|options| options.optional)
            .unwrap_or(false)
    }

    /// The value a missing optional pin resolves to: the stored literal when it decodes to a
    /// non-null value, otherwise the type default. See [`super::variable::default_value_for_type`].
    pub fn effective_default(&self, refs: &std::collections::HashMap<String, String>) -> Value {
        let stored = self
            .default_value
            .as_deref()
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok());
        let schema = self
            .schema
            .as_deref()
            .and_then(|schema| resolve_schema(schema, refs).ok());
        super::variable::effective_default(
            stored.as_ref(),
            &self.data_type,
            &self.value_type,
            schema,
        )
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

    /// The schema string `set_schema::<T>()` stores. Pins built from other metadata use it to
    /// carry exactly a native type's schema, because schema-enforcing pins compare the strings.
    pub fn schema_string_for<T: JsonSchema>() -> Option<String> {
        let schema = schema_for!(T);
        to_value(&schema).ok().and_then(|v| to_string(&v).ok())
    }

    pub fn set_schema<T: Serialize + JsonSchema>(&mut self) -> &mut Self {
        self.schema = Self::schema_string_for::<T>();
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
    fn geometry_connection_matrix_preserves_direction_containers_and_refs() {
        use flow_like_types::geometry::{GeometryKind, marker};
        let mut node = crate::flow::node::Node::new("geometry-test", "Geometry", "", "");
        let mut output = node
            .add_output_pin("out", "Out", "", super::VariableType::Geometry)
            .clone();
        let mut input = node
            .add_input_pin("in", "In", "", super::VariableType::Geometry)
            .clone();
        let refs = std::collections::HashMap::from([(
            "point-ref".into(),
            marker(GeometryKind::Point).into(),
        )]);
        let schemas: Vec<Option<String>> = std::iter::once(None)
            .chain(
                GeometryKind::ALL
                    .into_iter()
                    .map(|kind| Some(marker(kind).to_string())),
            )
            .collect();
        for source in &schemas {
            for target in &schemas {
                for enforce in [None, Some(false), Some(true)] {
                    output.schema = source.clone();
                    input.schema = target.clone();
                    input.options = Some(super::PinOptions {
                        enforce_schema: enforce,
                        ..Default::default()
                    });
                    assert_eq!(
                        super::geometry_pins_are_compatible(&output, &input, &refs).unwrap(),
                        target.is_none() || source == target
                    );
                }
            }
        }
        output.schema = Some("point-ref".into());
        input.schema = Some(marker(GeometryKind::Point).into());
        assert!(super::geometry_pins_are_compatible(&output, &input, &refs).unwrap());
        input.value_type = super::ValueType::Array;
        assert!(!super::geometry_pins_are_compatible(&output, &input, &refs).unwrap());
        input.data_type = super::VariableType::Generic;
        assert!(super::geometry_pins_are_compatible(&output, &input, &refs).unwrap());
        input.options.as_mut().unwrap().enforce_generic_value_type = Some(true);
        assert!(!super::geometry_pins_are_compatible(&output, &input, &refs).unwrap());
        output.schema = Some(r#"{"$id":"flow:geometry","x-geometry":"Triangle"}"#.into());
        assert!(super::geometry_pins_are_compatible(&output, &input, &refs).is_err());
    }

    #[test]
    fn effective_default_decodes_the_literal_and_resolves_schema_refs() {
        use flow_like_types::geometry::{GeometryKind, marker};
        use flow_like_types::json::json;
        let refs = std::collections::HashMap::from([(
            "multi-ref".to_string(),
            marker(GeometryKind::MultiPoint).to_string(),
        )]);
        let mut node = crate::flow::node::Node::new("default-test", "Default", "", "");
        let mut pin = node
            .add_output_pin("out", "Out", "", super::VariableType::Integer)
            .clone();
        assert!(!pin.is_optional());
        assert_eq!(pin.effective_default(&refs), json!(0));

        pin.set_options(super::PinOptions::new().set_optional(true).build());
        assert!(pin.is_optional());
        pin.set_default_value(Some(json!(4)));
        assert_eq!(pin.effective_default(&refs), json!(4));
        pin.set_default_value(Some(Value::Null));
        assert_eq!(pin.effective_default(&refs), json!(0));
        pin.default_value = Some(b"not json".to_vec());
        assert_eq!(pin.effective_default(&refs), json!(0));

        pin.set_default_value(None);
        pin.set_data_type(super::VariableType::Geometry);
        pin.schema = Some("multi-ref".into());
        assert_eq!(
            pin.effective_default(&refs),
            json!({"type": "MultiPoint", "coordinates": []})
        );
        pin.schema = Some(marker(GeometryKind::Point).into());
        assert_eq!(pin.effective_default(&refs), Value::Null);
        pin.set_value_type(super::ValueType::HashMap);
        assert_eq!(pin.effective_default(&refs), json!({}));
    }

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
