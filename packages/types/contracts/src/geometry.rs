//! Geometry values use the two-dimensional WGS 84 GeoJSON profile.
//!
//! Positions are longitude, latitude. Null represents an unset value and is not a
//! geometry. Empty multi-geometries and collections are supported; empty Point,
//! LineString, Polygon, and empty components inside multi-geometries are rejected.
//! Shape validation does not establish topological validity.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, str::FromStr};

pub const GEOMETRY_SCHEMA_ID: &str = "flow:geometry";
pub const MAX_GEOMETRY_DEPTH: usize = 32;
pub const MAX_GEOMETRY_POSITIONS: usize = 100_000;
pub const MAX_GEOMETRY_MEMBERS: usize = 1_000_000;
pub const MAX_GEOMETRY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum GeometryKind {
    Point,
    LineString,
    Polygon,
    MultiPoint,
    MultiLineString,
    MultiPolygon,
    GeometryCollection,
}

impl GeometryKind {
    pub const ALL: [Self; 7] = [
        Self::Point,
        Self::LineString,
        Self::Polygon,
        Self::MultiPoint,
        Self::MultiLineString,
        Self::MultiPolygon,
        Self::GeometryCollection,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Point => "Point",
            Self::LineString => "LineString",
            Self::Polygon => "Polygon",
            Self::MultiPoint => "MultiPoint",
            Self::MultiLineString => "MultiLineString",
            Self::MultiPolygon => "MultiPolygon",
            Self::GeometryCollection => "GeometryCollection",
        }
    }
}

impl fmt::Display for GeometryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GeometryKind {
    type Err = GeometryError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| GeometryError(format!("Unknown geometry kind: {value}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeometryError(pub String);
impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GeometryError {}

pub const fn marker(kind: GeometryKind) -> &'static str {
    match kind {
        GeometryKind::Point => r#"{"$id":"flow:geometry","x-geometry":"Point"}"#,
        GeometryKind::LineString => r#"{"$id":"flow:geometry","x-geometry":"LineString"}"#,
        GeometryKind::Polygon => r#"{"$id":"flow:geometry","x-geometry":"Polygon"}"#,
        GeometryKind::MultiPoint => r#"{"$id":"flow:geometry","x-geometry":"MultiPoint"}"#,
        GeometryKind::MultiLineString => {
            r#"{"$id":"flow:geometry","x-geometry":"MultiLineString"}"#
        }
        GeometryKind::MultiPolygon => r#"{"$id":"flow:geometry","x-geometry":"MultiPolygon"}"#,
        GeometryKind::GeometryCollection => {
            r#"{"$id":"flow:geometry","x-geometry":"GeometryCollection"}"#
        }
    }
}

/// Parse a resolved schema. Ordinary schemas are not geometry markers. References
/// must be resolved by the caller. Malformed JSON and malformed markers fail.
pub fn kind_from_schema(schema: &str) -> Result<Option<GeometryKind>, GeometryError> {
    let value: Value = serde_json::from_str(schema)
        .map_err(|err| GeometryError(format!("Invalid geometry schema JSON: {err}")))?;
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if object.get("$id").and_then(Value::as_str) != Some(GEOMETRY_SCHEMA_ID)
        && !object.contains_key("x-geometry")
    {
        return Ok(None);
    }
    if object.len() != 2 || object.get("$id").and_then(Value::as_str) != Some(GEOMETRY_SCHEMA_ID) {
        return Err(GeometryError(
            "Geometry subtype schema must contain only $id=flow:geometry and x-geometry".into(),
        ));
    }
    let kind = object
        .get("x-geometry")
        .and_then(Value::as_str)
        .ok_or_else(|| GeometryError("Geometry subtype marker requires x-geometry".into()))?;
    Ok(Some(kind.parse()?))
}

/// Subtype assignment is directional. Any geometry cannot narrow without validation.
pub fn compatible(source: Option<GeometryKind>, target: Option<GeometryKind>) -> bool {
    target.is_none() || source == target
}

pub fn parse_geometry(raw: &str, kind: Option<GeometryKind>) -> Result<Value, GeometryError> {
    if raw.len() > MAX_GEOMETRY_BYTES {
        return Err(GeometryError("Geometry exceeds the byte limit".into()));
    }
    let value = serde_json::from_str(raw)
        .map_err(|err| GeometryError(format!("Invalid GeoJSON: {err}")))?;
    validate_geometry(&value, kind)?;
    Ok(value)
}

pub fn validate_geometry(value: &Value, kind: Option<GeometryKind>) -> Result<(), GeometryError> {
    // Bound foreign members as well as coordinates before recursing through geometry.
    let mut stack = vec![(value, 0usize)];
    let mut members = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        members += 1;
        if depth > MAX_GEOMETRY_DEPTH || members > MAX_GEOMETRY_MEMBERS {
            return Err(GeometryError(
                "Geometry exceeds the depth or member limit".into(),
            ));
        }
        match value {
            Value::Array(values) => stack.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => {
                bytes += values.keys().map(String::len).sum::<usize>();
                stack.extend(values.values().map(|value| (value, depth + 1)));
            }
            Value::String(value) => bytes += value.len(),
            _ => bytes += 1,
        }
        if bytes > MAX_GEOMETRY_BYTES {
            return Err(GeometryError("Geometry exceeds the byte limit".into()));
        }
    }
    if serde_json::to_vec(value)
        .map_err(|err| GeometryError(err.to_string()))?
        .len()
        > MAX_GEOMETRY_BYTES
    {
        return Err(GeometryError("Geometry exceeds the byte limit".into()));
    }
    let mut positions = 0;
    validate_object(value, kind, &mut positions)
}

fn array(value: &Value) -> Result<&Vec<Value>, GeometryError> {
    value.as_array().ok_or_else(|| {
        GeometryError("Geometry coordinates must have the expected array nesting".into())
    })
}

fn position(value: &Value, count: &mut usize) -> Result<(), GeometryError> {
    *count += 1;
    if *count > MAX_GEOMETRY_POSITIONS {
        return Err(GeometryError("Geometry exceeds the position limit".into()));
    }
    let coordinates = array(value)?;
    if coordinates.len() != 2 {
        return Err(GeometryError("A position requires exactly longitude and latitude; Z/M dimensions and empty positions are unsupported".into()));
    }
    for (coordinate, limit) in coordinates.iter().zip([180.0, 90.0]) {
        let number = coordinate
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| GeometryError("Geometry coordinates must be finite numbers".into()))?;
        if !(-limit..=limit).contains(&number) {
            return Err(GeometryError(
                "Position is outside WGS 84 longitude [-180,180] or latitude [-90,90]".into(),
            ));
        }
    }
    Ok(())
}

fn line(value: &Value, ring: bool, count: &mut usize) -> Result<(), GeometryError> {
    let positions = array(value)?;
    if positions.len() < if ring { 4 } else { 2 } {
        return Err(GeometryError(
            if ring {
                "A polygon ring requires at least four positions"
            } else {
                "A LineString requires at least two positions"
            }
            .into(),
        ));
    }
    for value in positions {
        position(value, count)?;
    }
    if ring {
        let first = positions[0].as_array().unwrap();
        let last = positions.last().unwrap().as_array().unwrap();
        if first
            .iter()
            .zip(last)
            .any(|(a, b)| a.as_f64() != b.as_f64())
        {
            return Err(GeometryError("Polygon rings must be closed".into()));
        }
    }
    Ok(())
}

fn polygon(value: &Value, count: &mut usize) -> Result<(), GeometryError> {
    let rings = array(value)?;
    if rings.is_empty() {
        return Err(GeometryError(
            "Empty Polygon is unsupported; use an empty MultiPolygon".into(),
        ));
    }
    for ring in rings {
        line(ring, true, count)?;
    }
    Ok(())
}

fn validate_object(
    value: &Value,
    expected: Option<GeometryKind>,
    positions: &mut usize,
) -> Result<(), GeometryError> {
    let object = value.as_object().ok_or_else(|| {
        let received = match value {
            Value::Null => "null (unset)",
            Value::Bool(_) => "a boolean",
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Array(_) => "an array",
            Value::Object(_) => unreachable!(),
        };
        GeometryError(format!(
            "Geometry requires a GeoJSON geometry object; received {received}"
        ))
    })?;
    if object.contains_key("crs") {
        return Err(GeometryError(
            "Alternate CRS declarations are unsupported; Geometry uses WGS 84 longitude, latitude"
                .into(),
        ));
    }
    let kind: GeometryKind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| GeometryError("GeoJSON geometry requires a type".into()))?
        .parse()?;
    if expected.is_some_and(|expected| expected != kind) {
        return Err(GeometryError(format!(
            "Expected {}, received {kind}",
            expected.unwrap()
        )));
    }
    if let Some(bbox) = object.get("bbox") {
        let bounds = array(bbox)?;
        if bounds.len() != 4 {
            return Err(GeometryError(
                "A geometry bbox requires four coordinates".into(),
            ));
        }
        position(&Value::Array(bounds[..2].to_vec()), &mut 0)?;
        position(&Value::Array(bounds[2..].to_vec()), &mut 0)?;
        if bounds[1].as_f64() > bounds[3].as_f64() {
            return Err(GeometryError(
                "Geometry bbox south must not exceed north".into(),
            ));
        }
    }
    if kind == GeometryKind::GeometryCollection {
        if object.contains_key("coordinates") {
            return Err(GeometryError(
                "GeometryCollection uses geometries, not coordinates".into(),
            ));
        }
        for child in array(
            object
                .get("geometries")
                .ok_or_else(|| GeometryError("GeometryCollection requires geometries".into()))?,
        )? {
            validate_object(child, None, positions)?;
        }
        return Ok(());
    }
    if object.contains_key("geometries") {
        return Err(GeometryError(
            "Only GeometryCollection may contain geometries".into(),
        ));
    }
    let coordinates = object
        .get("coordinates")
        .ok_or_else(|| GeometryError("Geometry requires coordinates".into()))?;
    match kind {
        GeometryKind::Point => position(coordinates, positions)?,
        GeometryKind::LineString => line(coordinates, false, positions)?,
        GeometryKind::Polygon => polygon(coordinates, positions)?,
        GeometryKind::MultiPoint => {
            for value in array(coordinates)? {
                position(value, positions)?;
            }
        }
        GeometryKind::MultiLineString => {
            for value in array(coordinates)? {
                line(value, false, positions)?;
            }
        }
        GeometryKind::MultiPolygon => {
            for value in array(coordinates)? {
                polygon(value, positions)?;
            }
        }
        GeometryKind::GeometryCollection => unreachable!(),
    }
    Ok(())
}

/// Validate then normalize polygon winding. Foreign members and bbox are retained.
pub fn canonicalize_geometry(
    value: &Value,
    kind: Option<GeometryKind>,
) -> Result<Value, GeometryError> {
    validate_geometry(value, kind)?;
    let mut value = value.clone();
    normalize_winding(&mut value);
    Ok(value)
}

fn normalize_winding(value: &mut Value) {
    match value["type"].as_str().unwrap() {
        "Polygon" => normalize_polygon(value.get_mut("coordinates").unwrap()),
        "MultiPolygon" => {
            for polygon in value["coordinates"].as_array_mut().unwrap() {
                normalize_polygon(polygon);
            }
        }
        "GeometryCollection" => {
            for geometry in value["geometries"].as_array_mut().unwrap() {
                normalize_winding(geometry);
            }
        }
        _ => (),
    }
}

fn normalize_polygon(value: &mut Value) {
    for (index, ring) in value.as_array_mut().unwrap().iter_mut().enumerate() {
        let ring = ring.as_array_mut().unwrap();
        let signed_area: f64 = ring
            .windows(2)
            .map(|pair| {
                let x1 = pair[0][0].as_f64().unwrap();
                let y1 = pair[0][1].as_f64().unwrap();
                let x2 = pair[1][0].as_f64().unwrap();
                let y2 = pair[1][1].as_f64().unwrap();
                // GeoJSON line segments interpolate longitude directly, including at the antimeridian.
                x1 * y2 - x2 * y1
            })
            .sum();
        if (index == 0 && signed_area < 0.0) || (index != 0 && signed_area > 0.0) {
            ring.reverse();
        }
    }
}

/// JSON Schema expresses shape and coordinate bounds. Ring closure, depth, and
/// total position limits additionally require `validate_geometry` at runtime.
pub fn geometry_json_schema(kind: Option<GeometryKind>) -> Value {
    let position = json!({"type":"array","prefixItems":[
        {"type":"number","minimum":-180,"maximum":180},
        {"type":"number","minimum":-90,"maximum":90}],"items":false,"minItems":2,"maxItems":2});
    let line = json!({"type":"array","items":position,"minItems":2});
    let ring = json!({"type":"array","items":position,"minItems":4});
    let polygon = json!({"type":"array","items":ring,"minItems":1});
    let variants: Vec<Value> = GeometryKind::ALL.into_iter().map(|kind| {
        if kind == GeometryKind::GeometryCollection {
            return json!({"type":"object","properties":{"type":{"const":kind.as_str()},"geometries":{"type":"array","items":{"$ref":"#/$defs/geometry"}},"coordinates":false,"crs":false},"required":["type","geometries"]});
        }
        let coordinates = match kind {
            GeometryKind::Point => position.clone(),
            GeometryKind::LineString => line.clone(),
            GeometryKind::Polygon => polygon.clone(),
            GeometryKind::MultiPoint => json!({"type":"array","items":position}),
            GeometryKind::MultiLineString => json!({"type":"array","items":line}),
            GeometryKind::MultiPolygon => json!({"type":"array","items":polygon}),
            GeometryKind::GeometryCollection => unreachable!(),
        };
        json!({"type":"object","properties":{"type":{"const":kind.as_str()},"coordinates":coordinates,"geometries":false,"crs":false},"required":["type","coordinates"]})
    }).collect();
    let root = kind
        .map(|kind| {
            variants[GeometryKind::ALL
                .iter()
                .position(|candidate| *candidate == kind)
                .unwrap()]
            .clone()
        })
        .unwrap_or_else(|| json!({"oneOf":variants}));
    let mut root = root;
    root["$id"] = json!(format!(
        "urn:flow-like:geometry:v1:{}",
        kind.map(GeometryKind::as_str).unwrap_or("Any")
    ));
    root["x-flow-like-type"] = json!("geometry");
    if let Some(kind) = kind {
        root["x-geometry"] = json!(kind.as_str());
    }
    root["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    root["$defs"] = json!({"geometry":{"oneOf":variants}});
    root["description"] = json!(
        "WGS 84 GeoJSON geometry. Each position is [longitude, latitude]. Exactly two finite coordinates are required. Polygon rings must be closed."
    );
    root
}

/// Projection for tool providers that support only type, properties, required,
/// enum, and items. Coordinate bounds, ring closure, recursive collection members,
/// and generic coordinate nesting remain enforced by `validate_geometry`.
pub fn geometry_tool_schema(kind: Option<GeometryKind>) -> Value {
    let kinds: Vec<&str> = kind.map(|kind| vec![kind.as_str()]).unwrap_or_else(|| {
        GeometryKind::ALL
            .into_iter()
            .map(GeometryKind::as_str)
            .collect()
    });
    let mut schema = json!({"type":"object", "properties": {
        "type":{"type":"string","enum":kinds}
    },"required":["type"],"description":"GeoJSON geometry in WGS 84. Positions are exactly [longitude, latitude], longitude -180 to 180 and latitude -90 to 90. Polygon rings must be closed. No Z/M dimensions or CRS declaration."});
    if kind != Some(GeometryKind::GeometryCollection) {
        let depth = match kind {
            Some(GeometryKind::Point) => 1,
            Some(GeometryKind::LineString | GeometryKind::MultiPoint) => 2,
            Some(GeometryKind::Polygon | GeometryKind::MultiLineString) => 3,
            Some(GeometryKind::MultiPolygon) => 4,
            _ => 0,
        };
        let mut coordinates = json!({"type":"number"});
        for _ in 0..depth {
            coordinates = json!({"type":"array","items":coordinates});
        }
        if depth == 0 {
            coordinates = json!({"type":"array"});
        }
        schema["properties"]["coordinates"] = coordinates;
        if kind.is_some() {
            schema["required"]
                .as_array_mut()
                .unwrap()
                .push(json!("coordinates"));
        }
    }
    if kind.is_none() || kind == Some(GeometryKind::GeometryCollection) {
        schema["properties"]["geometries"] = json!({"type":"array","items":{"type":"object"},"description":"GeoJSON geometry objects; required for GeometryCollection."});
        if kind.is_some() {
            schema["required"]
                .as_array_mut()
                .unwrap()
                .push(json!("geometries"));
        }
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point() -> Value {
        json!({"type":"Point","coordinates":[13.405,52.52]})
    }

    #[test]
    fn markers_and_direction_are_frozen() {
        for kind in GeometryKind::ALL {
            assert_eq!(kind_from_schema(marker(kind)).unwrap(), Some(kind));
            assert!(compatible(Some(kind), None));
            assert!(!compatible(None, Some(kind)));
            for other in GeometryKind::ALL {
                assert_eq!(compatible(Some(kind), Some(other)), kind == other);
            }
        }
        assert!(compatible(None, None));
        assert_eq!(kind_from_schema(r#"{"type":"object"}"#).unwrap(), None);
        for marker in [
            r#"{"$id":"flow:geometry"}"#,
            r#"{"x-geometry":"Point"}"#,
            r#"{"$id":"flow:geometry","x-geometry":"Triangle"}"#,
            r#"{"$id":"flow:geometry","x-geometry":"Point","type":"object"}"#,
        ] {
            assert!(kind_from_schema(marker).is_err());
        }
    }

    #[test]
    fn non_object_errors_identify_the_value_type_without_exposing_values() {
        for (value, received) in [
            (Value::Null, "null (unset)"),
            (json!("private text"), "a string"),
            (json!(false), "a boolean"),
            (json!(42), "a number"),
            (json!([]), "an array"),
        ] {
            assert_eq!(
                validate_geometry(&value, None).unwrap_err().to_string(),
                format!("Geometry requires a GeoJSON geometry object; received {received}")
            );
        }
    }

    #[test]
    fn all_shapes_and_empty_policy() {
        let ring = json!([[0, 0], [1, 0], [1, 1], [0, 0]]);
        for value in [
            point(),
            json!({"type":"LineString","coordinates":[[0,0],[180,90]]}),
            json!({"type":"Polygon","coordinates":[ring]}),
            json!({"type":"MultiPoint","coordinates":[[13,52]]}),
            json!({"type":"MultiLineString","coordinates":[[[0,0],[1,1]]]}),
            json!({"type":"MultiPolygon","coordinates":[[ring]]}),
            json!({"type":"GeometryCollection","geometries":[point()]}),
        ] {
            validate_geometry(&value, None).unwrap();
        }
        for kind in GeometryKind::ALL {
            let field = if kind == GeometryKind::GeometryCollection {
                "geometries"
            } else {
                "coordinates"
            };
            let value = json!({"type":kind.as_str(),field:[]});
            assert_eq!(
                validate_geometry(&value, None).is_ok(),
                !matches!(
                    kind,
                    GeometryKind::Point | GeometryKind::LineString | GeometryKind::Polygon
                )
            );
        }
        for value in [
            Value::Null,
            json!({"type":"Feature","geometry":point()}),
            json!({"type":"MultiLineString","coordinates":[[]]}),
            json!({"type":"MultiPolygon","coordinates":[[]]}),
            json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}),
            json!({"type":"Point","coordinates":[0,0,4]}),
            json!({"type":"Point","coordinates":[52.52,113.405]}),
            json!({"type":"Point","coordinates":[181,0]}),
            json!({"type":"Point","coordinates":[null,0]}),
            json!({"type":"Point","coordinates":[0,0],"crs":null}),
        ] {
            assert!(validate_geometry(&value, None).is_err(), "{value}");
        }
        assert!(validate_geometry(&point(), Some(GeometryKind::Polygon)).is_err());
        assert!(parse_geometry(r#"{"type":"Point","coordinates":[1e400,0]}"#, None).is_err());
    }

    #[test]
    fn depth_is_bounded_in_foreign_members_and_collections() {
        let mut value = point();
        for _ in 0..MAX_GEOMETRY_DEPTH {
            value = json!({"type":"GeometryCollection","geometries":[value]});
        }
        assert!(validate_geometry(&value, None).is_err());
        let mut foreign = Value::Null;
        for _ in 0..=MAX_GEOMETRY_DEPTH {
            foreign = json!([foreign]);
        }
        let mut value = point();
        value["foreign"] = foreign;
        assert!(validate_geometry(&value, None).is_err());
    }

    #[test]
    fn winding_is_idempotent_and_foreign_members_survive() {
        let value = json!({"type":"Polygon","coordinates":[[[0,0],[0,1],[1,1],[0,0]]],"bbox":[0,0,1,1],"label":"kept"});
        let canonical = canonicalize_geometry(&value, None).unwrap();
        assert_eq!(canonical["label"], "kept");
        assert_eq!(canonical["bbox"], value["bbox"]);
        assert_eq!(
            canonical["coordinates"][0],
            json!([[0, 0], [1, 1], [0, 1], [0, 0]])
        );
        assert_eq!(canonicalize_geometry(&canonical, None).unwrap(), canonical);
        validate_geometry(
            &json!({"type":"Point","coordinates":[180,90],"bbox":[170,-90,-170,90]}),
            None,
        )
        .unwrap();
    }
}
