//! Native geometry construction and component access nodes.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    geometry::{GeometryKind, marker},
};

#[cfg(feature = "execute")]
use flow_like_geometry::to_geo;
#[cfg(feature = "execute")]
use flow_like_types::{
    Result, Value, anyhow, bail,
    geometry::{MAX_GEOMETRY_POSITIONS, canonicalize_geometry},
    json::{Map, json},
};
#[cfg(feature = "execute")]
use std::collections::HashMap;

#[cfg(feature = "execute")]
use super::{add_estimated_geometry_positions, check_topology_validation, cpu};

#[cfg(feature = "execute")]
const MAX_COLLECTION_BOUNDARY_POLYGON_PAIRS: usize = 2_000_000;

#[derive(Clone, Copy, Debug)]
enum Operation {
    MakeLineString,
    MakePolygon,
    MakeMultiPoint,
    MakeMultiLineString,
    MakeMultiPolygon,
    MakeGeometryCollection,
    MakeEnvelope,
    Parts,
    GeometryN,
    NumGeometries,
    Points,
    PointN,
    StartPoint,
    EndPoint,
    ExteriorRing,
    InteriorRings,
    NumInteriorRings,
    Boundary,
    ToMulti,
    ToSingle,
}

fn geometry_input<'a>(
    node: &'a mut Node,
    name: &str,
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    let pin = node.add_input_pin(name, name, description, VariableType::Geometry);
    pin.schema = kind.map(|kind| marker(kind).to_string());
    pin
}

fn geometry_output<'a>(
    node: &'a mut Node,
    name: &str,
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    let pin = node.add_output_pin(name, name, description, VariableType::Geometry);
    pin.schema = kind.map(|kind| marker(kind).to_string());
    pin
}

fn geometry_array_input<'a>(
    node: &'a mut Node,
    name: &str,
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    geometry_input(node, name, description, kind).set_value_type(ValueType::Array)
}

fn geometry_array_output<'a>(
    node: &'a mut Node,
    name: &str,
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    geometry_output(node, name, description, kind).set_value_type(ValueType::Array)
}

fn data_input<'a>(
    node: &'a mut Node,
    name: &str,
    description: &str,
    data_type: VariableType,
) -> &'a mut Pin {
    node.add_input_pin(name, name, description, data_type)
}

fn data_output<'a>(
    node: &'a mut Node,
    name: &str,
    description: &str,
    data_type: VariableType,
) -> &'a mut Pin {
    node.add_output_pin(name, name, description, data_type)
}

fn definition(operation: Operation) -> Node {
    use Operation::*;
    let (id, alias, title, description) = match operation {
        MakeLineString => (
            "geometry_make_line_string",
            "makeLineString",
            "Make LineString",
            "Creates a LineString from an ordered array of Point geometries. At least two Points are required.",
        ),
        MakePolygon => (
            "geometry_make_polygon",
            "makePolygon",
            "Make Polygon",
            "Creates a Polygon from an exterior LineString and optional interior LineString rings. Open rings are closed automatically.",
        ),
        MakeMultiPoint => (
            "geometry_make_multi_point",
            "makeMultiPoint",
            "Make MultiPoint",
            "Creates a MultiPoint from Point geometries. An empty input creates an empty MultiPoint.",
        ),
        MakeMultiLineString => (
            "geometry_make_multi_line_string",
            "makeMultiLineString",
            "Make MultiLineString",
            "Creates a MultiLineString from LineString geometries. An empty input creates an empty MultiLineString.",
        ),
        MakeMultiPolygon => (
            "geometry_make_multi_polygon",
            "makeMultiPolygon",
            "Make MultiPolygon",
            "Creates a MultiPolygon from Polygon geometries. An empty input creates an empty MultiPolygon.",
        ),
        MakeGeometryCollection => (
            "geometry_make_geometry_collection",
            "makeGeometryCollection",
            "Make GeometryCollection",
            "Creates a GeometryCollection from any validated geometries, including nested collections.",
        ),
        MakeEnvelope => (
            "geometry_make_envelope",
            "makeEnvelope",
            "Make Envelope",
            "Creates a rectangular Polygon from west, south, east and north WGS 84 bounds. The envelope must have positive width and height.",
        ),
        Parts => (
            "geometry_parts",
            "parts",
            "Geometry Parts",
            "Explodes a multi-geometry or GeometryCollection into its immediate members. A simple geometry returns a one-item array.",
        ),
        GeometryN => (
            "geometry_n",
            "geometryN",
            "Geometry N",
            "Returns one immediate geometry member by zero-based index. A simple geometry has one member at index zero.",
        ),
        NumGeometries => (
            "geometry_num_geometries",
            "numGeometries",
            "Number of Geometries",
            "Counts immediate members of a multi-geometry or GeometryCollection. A simple geometry has one member.",
        ),
        Points => (
            "geometry_points",
            "points",
            "Geometry Points",
            "Returns every coordinate position as a Point in traversal order, including polygon ring closing positions.",
        ),
        PointN => (
            "geometry_point_n",
            "pointN",
            "Geometry Point N",
            "Returns one coordinate position by zero-based traversal index, including polygon ring closing positions.",
        ),
        StartPoint => (
            "geometry_start_point",
            "startPoint",
            "LineString Start Point",
            "Returns the first Point of a LineString.",
        ),
        EndPoint => (
            "geometry_end_point",
            "endPoint",
            "LineString End Point",
            "Returns the last Point of a LineString.",
        ),
        ExteriorRing => (
            "geometry_exterior_ring",
            "exteriorRing",
            "Polygon Exterior Ring",
            "Returns a Polygon's exterior ring as a closed LineString.",
        ),
        InteriorRings => (
            "geometry_interior_rings",
            "interiorRings",
            "Polygon Interior Rings",
            "Returns a Polygon's interior rings as closed LineStrings.",
        ),
        NumInteriorRings => (
            "geometry_num_interior_rings",
            "numInteriorRings",
            "Number of Interior Rings",
            "Counts a Polygon's interior rings.",
        ),
        Boundary => (
            "geometry_boundary",
            "boundary",
            "Geometry Boundary",
            "Returns the planar topological boundary. Point boundaries are empty, line boundaries are endpoint MultiPoints and polygon boundaries are MultiLineStrings. GeometryCollections containing both lineal and polygonal members are unsupported.",
        ),
        ToMulti => (
            "geometry_to_multi",
            "toMulti",
            "Geometry to Multi",
            "Wraps a Point, LineString or Polygon in its corresponding multi-geometry. Existing multi-geometries pass through unchanged.",
        ),
        ToSingle => (
            "geometry_to_single",
            "toSingle",
            "Geometry to Single",
            "Unwraps a multi-geometry containing exactly one member. Simple geometries pass through unchanged.",
        ),
    };

    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");

    match operation {
        MakeLineString => {
            geometry_array_input(
                &mut node,
                "points",
                "Ordered Point geometries",
                Some(GeometryKind::Point),
            );
            geometry_output(
                &mut node,
                "geometry_out",
                "Constructed LineString",
                Some(GeometryKind::LineString),
            );
        }
        MakePolygon => {
            geometry_input(
                &mut node,
                "exterior",
                "Exterior ring as a LineString",
                Some(GeometryKind::LineString),
            );
            geometry_array_input(
                &mut node,
                "holes",
                "Interior rings as LineStrings",
                Some(GeometryKind::LineString),
            )
            .set_default_value(Some(flow_like_types::json::json!([])));
            geometry_output(
                &mut node,
                "geometry_out",
                "Constructed Polygon",
                Some(GeometryKind::Polygon),
            );
        }
        MakeMultiPoint => {
            geometry_array_input(
                &mut node,
                "points",
                "Point geometries",
                Some(GeometryKind::Point),
            )
            .set_default_value(Some(flow_like_types::json::json!([])));
            geometry_output(
                &mut node,
                "geometry_out",
                "Constructed MultiPoint",
                Some(GeometryKind::MultiPoint),
            );
        }
        MakeMultiLineString => {
            geometry_array_input(
                &mut node,
                "lines",
                "LineString geometries",
                Some(GeometryKind::LineString),
            )
            .set_default_value(Some(flow_like_types::json::json!([])));
            geometry_output(
                &mut node,
                "geometry_out",
                "Constructed MultiLineString",
                Some(GeometryKind::MultiLineString),
            );
        }
        MakeMultiPolygon => {
            geometry_array_input(
                &mut node,
                "polygons",
                "Polygon geometries",
                Some(GeometryKind::Polygon),
            )
            .set_default_value(Some(flow_like_types::json::json!([])));
            geometry_output(
                &mut node,
                "geometry_out",
                "Constructed MultiPolygon",
                Some(GeometryKind::MultiPolygon),
            );
        }
        MakeGeometryCollection => {
            geometry_array_input(&mut node, "geometries", "Member geometries", None)
                .set_default_value(Some(flow_like_types::json::json!([])));
            geometry_output(
                &mut node,
                "geometry_out",
                "Constructed GeometryCollection",
                Some(GeometryKind::GeometryCollection),
            );
        }
        MakeEnvelope => {
            for (name, description) in [
                ("west", "Minimum longitude in degrees"),
                ("south", "Minimum latitude in degrees"),
                ("east", "Maximum longitude in degrees"),
                ("north", "Maximum latitude in degrees"),
            ] {
                data_input(&mut node, name, description, VariableType::Float);
            }
            geometry_output(
                &mut node,
                "geometry_out",
                "Rectangular Polygon",
                Some(GeometryKind::Polygon),
            );
        }
        Parts => {
            geometry_input(&mut node, "geometry", "Geometry to explode", None);
            geometry_array_output(&mut node, "parts", "Immediate geometry members", None);
        }
        GeometryN => {
            geometry_input(&mut node, "geometry", "Geometry to inspect", None);
            data_input(
                &mut node,
                "index",
                "Zero-based member index",
                VariableType::Integer,
            );
            geometry_output(&mut node, "geometry_out", "Selected geometry member", None);
        }
        NumGeometries => {
            geometry_input(&mut node, "geometry", "Geometry to inspect", None);
            data_output(
                &mut node,
                "count",
                "Number of immediate members",
                VariableType::Integer,
            );
        }
        Points => {
            geometry_input(&mut node, "geometry", "Geometry to inspect", None);
            geometry_array_output(
                &mut node,
                "points",
                "Coordinate positions as Points",
                Some(GeometryKind::Point),
            );
        }
        PointN => {
            geometry_input(&mut node, "geometry", "Geometry to inspect", None);
            data_input(
                &mut node,
                "index",
                "Zero-based coordinate position index",
                VariableType::Integer,
            );
            geometry_output(
                &mut node,
                "geometry_out",
                "Selected coordinate Point",
                Some(GeometryKind::Point),
            );
        }
        StartPoint | EndPoint => {
            geometry_input(
                &mut node,
                "geometry",
                "LineString to inspect",
                Some(GeometryKind::LineString),
            );
            geometry_output(
                &mut node,
                "geometry_out",
                if matches!(operation, StartPoint) {
                    "First Point"
                } else {
                    "Last Point"
                },
                Some(GeometryKind::Point),
            );
        }
        ExteriorRing => {
            geometry_input(
                &mut node,
                "geometry",
                "Polygon to inspect",
                Some(GeometryKind::Polygon),
            );
            geometry_output(
                &mut node,
                "geometry_out",
                "Exterior ring as a LineString",
                Some(GeometryKind::LineString),
            );
        }
        InteriorRings => {
            geometry_input(
                &mut node,
                "geometry",
                "Polygon to inspect",
                Some(GeometryKind::Polygon),
            );
            geometry_array_output(
                &mut node,
                "rings",
                "Interior rings as LineStrings",
                Some(GeometryKind::LineString),
            );
        }
        NumInteriorRings => {
            geometry_input(
                &mut node,
                "geometry",
                "Polygon to inspect",
                Some(GeometryKind::Polygon),
            );
            data_output(
                &mut node,
                "count",
                "Number of interior rings",
                VariableType::Integer,
            );
        }
        Boundary | ToMulti | ToSingle => {
            geometry_input(&mut node, "geometry", "Input geometry", None);
            geometry_output(&mut node, "geometry_out", "Result geometry", None);
        }
    }

    if let Some(receiver) = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input)
        .min_by_key(|pin| pin.index)
        .filter(|pin| pin.data_type == VariableType::Geometry)
        .map(|pin| pin.name.clone())
    {
        node.set_receiver(&receiver);
    }
    node
}

#[cfg(feature = "execute")]
fn input<'a>(inputs: &'a Value, name: &str) -> Result<&'a Value> {
    inputs
        .get(name)
        .ok_or_else(|| anyhow!("Missing geometry input {name}"))
}

#[cfg(feature = "execute")]
fn number(inputs: &Value, name: &str) -> Result<f64> {
    let value = input(inputs, name)?
        .as_f64()
        .ok_or_else(|| anyhow!("{name} must be a finite number"))?;
    if !value.is_finite() {
        bail!("{name} must be finite");
    }
    Ok(value)
}

#[cfg(feature = "execute")]
fn index(inputs: &Value) -> Result<usize> {
    let value = input(inputs, "index")?
        .as_i64()
        .ok_or_else(|| anyhow!("index must be an integer"))?;
    usize::try_from(value).map_err(|_| anyhow!("index must be nonnegative"))
}

#[cfg(feature = "execute")]
fn geometry(value: &Value, kind: Option<GeometryKind>) -> Result<Value> {
    let value = canonicalize_geometry(value, kind)?;
    check_topology_validation(&to_geo(&value)?, "Invalid geometry topology")?;
    Ok(value)
}

#[cfg(feature = "execute")]
fn geometry_array(value: &Value, kind: Option<GeometryKind>) -> Result<Vec<Value>> {
    let values = value
        .as_array()
        .ok_or_else(|| anyhow!("Expected an array of geometries"))?;
    if values.len() > MAX_GEOMETRY_POSITIONS {
        bail!("Geometry array contains more than {MAX_GEOMETRY_POSITIONS} members");
    }
    let mut positions = 0usize;
    let mut members = 1usize;
    for value in values {
        add_estimated_geometry_positions(value, &mut positions, &mut members, "Geometry array")?;
    }
    let work = positions.saturating_add(values.len());
    cpu::map_ordered(values, work, |_, value| geometry(value, kind))
        .into_iter()
        .collect()
}

#[cfg(feature = "execute")]
fn geometry_result(value: Value) -> Result<Vec<(&'static str, Value)>> {
    Ok(vec![("geometry_out", geometry(&value, None)?)])
}

#[cfg(feature = "execute")]
fn coordinates(value: &Value) -> Result<Value> {
    value
        .get("coordinates")
        .cloned()
        .ok_or_else(|| anyhow!("Geometry has no coordinates"))
}

#[cfg(feature = "execute")]
fn same_position(a: &Value, b: &Value) -> bool {
    position_key(a).ok() == position_key(b).ok()
}

#[cfg(feature = "execute")]
fn closed_ring(line: &Value) -> Result<Value> {
    let line = geometry(line, Some(GeometryKind::LineString))?;
    let mut positions = coordinates(&line)?
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("LineString coordinates must be an array"))?;
    if !same_position(&positions[0], positions.last().unwrap()) {
        positions.push(positions[0].clone());
    }
    Ok(Value::Array(positions))
}

#[cfg(feature = "execute")]
fn immediate_parts(value: &Value) -> Result<Vec<Value>> {
    let value = geometry(value, None)?;
    let kind = value["type"]
        .as_str()
        .ok_or_else(|| anyhow!("Geometry requires a type"))?;
    match kind {
        "Point" | "LineString" | "Polygon" => Ok(vec![value]),
        "MultiPoint" | "MultiLineString" | "MultiPolygon" => {
            let child_kind = match kind {
                "MultiPoint" => "Point",
                "MultiLineString" => "LineString",
                "MultiPolygon" => "Polygon",
                _ => unreachable!(),
            };
            value["coordinates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|coordinates| {
                    geometry(
                        &json!({"type": child_kind, "coordinates": coordinates}),
                        None,
                    )
                })
                .collect()
        }
        "GeometryCollection" => value["geometries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| geometry(member, None))
            .collect(),
        _ => unreachable!(),
    }
}

#[cfg(feature = "execute")]
fn collect_positions(value: &Value, output: &mut Vec<Value>) {
    match value["type"].as_str().unwrap() {
        "Point" => output.push(value["coordinates"].clone()),
        "LineString" | "MultiPoint" => {
            output.extend(value["coordinates"].as_array().unwrap().iter().cloned());
        }
        "Polygon" | "MultiLineString" => {
            for line in value["coordinates"].as_array().unwrap() {
                output.extend(line.as_array().unwrap().iter().cloned());
            }
        }
        "MultiPolygon" => {
            for polygon in value["coordinates"].as_array().unwrap() {
                for line in polygon.as_array().unwrap() {
                    output.extend(line.as_array().unwrap().iter().cloned());
                }
            }
        }
        "GeometryCollection" => {
            for member in value["geometries"].as_array().unwrap() {
                collect_positions(member, output);
            }
        }
        _ => unreachable!(),
    }
}

#[cfg(feature = "execute")]
fn points(value: &Value) -> Result<Vec<Value>> {
    let value = geometry(value, None)?;
    let mut positions = Vec::new();
    collect_positions(&value, &mut positions);
    positions
        .into_iter()
        .map(|coordinates| geometry(&json!({"type":"Point", "coordinates":coordinates}), None))
        .collect()
}

#[cfg(feature = "execute")]
fn position_key(value: &Value) -> Result<(u64, u64)> {
    let coordinates = value
        .as_array()
        .filter(|coordinates| coordinates.len() == 2)
        .ok_or_else(|| anyhow!("A position requires longitude and latitude"))?;
    let bits = |value: &Value| -> Result<u64> {
        let number = value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| anyhow!("Position coordinates must be finite numbers"))?;
        Ok(if number == 0.0 { 0.0 } else { number }.to_bits())
    };
    Ok((bits(&coordinates[0])?, bits(&coordinates[1])?))
}

#[cfg(feature = "execute")]
fn line_boundaries<'a>(lines: impl IntoIterator<Item = &'a Value>) -> Result<Vec<Value>> {
    let mut endpoints: HashMap<(u64, u64), (usize, usize, Value)> = HashMap::new();
    let mut order = 0usize;
    for line in lines {
        let positions = line
            .as_array()
            .ok_or_else(|| anyhow!("LineString coordinates must be an array"))?;
        for endpoint in [&positions[0], positions.last().unwrap()] {
            let key = position_key(endpoint)?;
            let entry = endpoints
                .entry(key)
                .or_insert_with(|| (order, 0, endpoint.clone()));
            entry.1 += 1;
            order += 1;
        }
    }
    let mut endpoints: Vec<_> = endpoints
        .into_values()
        .filter(|(_, count, _)| count % 2 == 1)
        .collect();
    endpoints.sort_by_key(|(order, _, _)| *order);
    Ok(endpoints
        .into_iter()
        .map(|(_, _, position)| position)
        .collect())
}

#[cfg(feature = "execute")]
#[derive(Default)]
struct CollectionBoundaryComponents {
    lines: Vec<Value>,
    polygons: Vec<Value>,
}

#[cfg(feature = "execute")]
fn collect_collection_boundary_components(
    value: &Value,
    components: &mut CollectionBoundaryComponents,
) {
    match value["type"].as_str().unwrap() {
        "Point" | "MultiPoint" => {}
        "LineString" => components.lines.push(value["coordinates"].clone()),
        "MultiLineString" => components
            .lines
            .extend(value["coordinates"].as_array().unwrap().iter().cloned()),
        "Polygon" => components.polygons.push(value["coordinates"].clone()),
        "MultiPolygon" => components
            .polygons
            .extend(value["coordinates"].as_array().unwrap().iter().cloned()),
        "GeometryCollection" => {
            for member in value["geometries"].as_array().unwrap() {
                collect_collection_boundary_components(member, components);
            }
        }
        _ => unreachable!(),
    }
}

#[cfg(feature = "execute")]
fn collection_boundary(value: &Value) -> Result<Value> {
    let mut components = CollectionBoundaryComponents::default();
    collect_collection_boundary_components(value, &mut components);
    if !components.lines.is_empty() && !components.polygons.is_empty() {
        bail!(
            "Boundary does not support GeometryCollections containing both lineal and polygonal members; extract homogeneous parts first"
        );
    }
    if !components.polygons.is_empty() {
        let polygon_pairs = components
            .polygons
            .len()
            .saturating_mul(components.polygons.len().saturating_sub(1))
            / 2;
        if polygon_pairs > MAX_COLLECTION_BOUNDARY_POLYGON_PAIRS {
            bail!(
                "Polygonal GeometryCollection boundary requires {polygon_pairs} topology comparisons, exceeding the limit of {MAX_COLLECTION_BOUNDARY_POLYGON_PAIRS}"
            );
        }
        let polygons = geometry(
            &json!({"type":"MultiPolygon", "coordinates":components.polygons}),
            Some(GeometryKind::MultiPolygon),
        )
        .map_err(|error| {
            anyhow!(
                "Polygonal GeometryCollection boundary requires members that form a valid MultiPolygon: {error}"
            )
        })?;
        let mut rings = Vec::new();
        for polygon in polygons["coordinates"].as_array().unwrap() {
            rings.extend(polygon.as_array().unwrap().iter().cloned());
        }
        return geometry(
            &json!({"type":"MultiLineString", "coordinates":rings}),
            None,
        );
    }
    if !components.lines.is_empty() {
        return geometry(
            &json!({
                "type":"MultiPoint",
                "coordinates":line_boundaries(&components.lines)?
            }),
            None,
        );
    }
    geometry(&json!({"type":"GeometryCollection", "geometries":[]}), None)
}

#[cfg(feature = "execute")]
fn boundary(value: &Value) -> Result<Value> {
    let value = geometry(value, None)?;
    let result = match value["type"].as_str().unwrap() {
        "Point" | "MultiPoint" => json!({"type":"GeometryCollection", "geometries":[]}),
        "LineString" => json!({
            "type":"MultiPoint",
            "coordinates":line_boundaries(std::iter::once(&value["coordinates"]))?
        }),
        "MultiLineString" => json!({
            "type":"MultiPoint",
            "coordinates":line_boundaries(value["coordinates"].as_array().unwrap())?
        }),
        "Polygon" => json!({
            "type":"MultiLineString",
            "coordinates":value["coordinates"].clone()
        }),
        "MultiPolygon" => {
            let mut rings = Vec::new();
            for polygon in value["coordinates"].as_array().unwrap() {
                rings.extend(polygon.as_array().unwrap().iter().cloned());
            }
            json!({"type":"MultiLineString", "coordinates":rings})
        }
        "GeometryCollection" => return collection_boundary(&value),
        _ => unreachable!(),
    };
    geometry(&result, None)
}

#[cfg(feature = "execute")]
fn change_simple_multi(value: &Value, to_multi: bool) -> Result<Value> {
    let mut value = geometry(value, None)?;
    let kind = value["type"].as_str().unwrap();
    let (target, wrap) = if to_multi {
        match kind {
            "Point" => ("MultiPoint", true),
            "LineString" => ("MultiLineString", true),
            "Polygon" => ("MultiPolygon", true),
            "MultiPoint" | "MultiLineString" | "MultiPolygon" => return Ok(value),
            "GeometryCollection" => bail!("GeometryCollection has no corresponding multi type"),
            _ => unreachable!(),
        }
    } else {
        match kind {
            "Point" | "LineString" | "Polygon" => return Ok(value),
            "MultiPoint" => ("Point", false),
            "MultiLineString" => ("LineString", false),
            "MultiPolygon" => ("Polygon", false),
            "GeometryCollection" => bail!("GeometryCollection is not a multi-geometry"),
            _ => unreachable!(),
        }
    };
    let object = value.as_object_mut().unwrap();
    let coordinates = object.get_mut("coordinates").unwrap();
    *coordinates = if wrap {
        Value::Array(vec![coordinates.clone()])
    } else {
        let members = coordinates.as_array().unwrap();
        if members.len() != 1 {
            bail!(
                "Multi-geometry must contain exactly one member, received {}",
                members.len()
            );
        }
        members[0].clone()
    };
    object.insert("type".into(), Value::String(target.into()));
    geometry(&value, None)
}

#[cfg(feature = "execute")]
fn execute(operation: Operation, inputs: &Value) -> Result<Vec<(&'static str, Value)>> {
    use Operation::*;
    match operation {
        MakeLineString => {
            let points = geometry_array(input(inputs, "points")?, Some(GeometryKind::Point))?;
            let coordinates = points.iter().map(coordinates).collect::<Result<Vec<_>>>()?;
            geometry_result(json!({"type":"LineString", "coordinates":coordinates}))
        }
        MakePolygon => {
            let mut rings = vec![closed_ring(input(inputs, "exterior")?)?];
            rings.extend(
                geometry_array(input(inputs, "holes")?, Some(GeometryKind::LineString))?
                    .iter()
                    .map(closed_ring)
                    .collect::<Result<Vec<_>>>()?,
            );
            geometry_result(json!({"type":"Polygon", "coordinates":rings}))
        }
        MakeMultiPoint | MakeMultiLineString | MakeMultiPolygon => {
            let (name, source_kind, target_kind) = match operation {
                MakeMultiPoint => ("points", GeometryKind::Point, "MultiPoint"),
                MakeMultiLineString => ("lines", GeometryKind::LineString, "MultiLineString"),
                MakeMultiPolygon => ("polygons", GeometryKind::Polygon, "MultiPolygon"),
                _ => unreachable!(),
            };
            let members = geometry_array(input(inputs, name)?, Some(source_kind))?;
            let coordinates = members
                .iter()
                .map(coordinates)
                .collect::<Result<Vec<_>>>()?;
            geometry_result(json!({"type":target_kind, "coordinates":coordinates}))
        }
        MakeGeometryCollection => {
            let geometries = geometry_array(input(inputs, "geometries")?, None)?;
            geometry_result(json!({"type":"GeometryCollection", "geometries":geometries}))
        }
        MakeEnvelope => {
            let west = number(inputs, "west")?;
            let south = number(inputs, "south")?;
            let east = number(inputs, "east")?;
            let north = number(inputs, "north")?;
            if west >= east {
                bail!("Envelope west must be less than east");
            }
            if south >= north {
                bail!("Envelope south must be less than north");
            }
            geometry_result(json!({
                "type":"Polygon",
                "coordinates":[[[west,south],[east,south],[east,north],[west,north],[west,south]]]
            }))
        }
        Parts => Ok(vec![(
            "parts",
            Value::Array(immediate_parts(input(inputs, "geometry")?)?),
        )]),
        GeometryN => {
            let index = index(inputs)?;
            let parts = immediate_parts(input(inputs, "geometry")?)?;
            let value = parts.get(index).cloned().ok_or_else(|| {
                anyhow!(
                    "Geometry index {index} is out of range for {} members",
                    parts.len()
                )
            })?;
            geometry_result(value)
        }
        NumGeometries => Ok(vec![(
            "count",
            json!(immediate_parts(input(inputs, "geometry")?)?.len()),
        )]),
        Points => Ok(vec![(
            "points",
            Value::Array(points(input(inputs, "geometry")?)?),
        )]),
        PointN => {
            let index = index(inputs)?;
            let points = points(input(inputs, "geometry")?)?;
            let value = points.get(index).cloned().ok_or_else(|| {
                anyhow!(
                    "Point index {index} is out of range for {} positions",
                    points.len()
                )
            })?;
            geometry_result(value)
        }
        StartPoint | EndPoint => {
            let line = geometry(input(inputs, "geometry")?, Some(GeometryKind::LineString))?;
            let positions = line["coordinates"].as_array().unwrap();
            let position = if matches!(operation, StartPoint) {
                &positions[0]
            } else {
                positions.last().unwrap()
            };
            geometry_result(json!({"type":"Point", "coordinates":position}))
        }
        ExteriorRing => {
            let polygon = geometry(input(inputs, "geometry")?, Some(GeometryKind::Polygon))?;
            geometry_result(json!({
                "type":"LineString",
                "coordinates":polygon["coordinates"][0]
            }))
        }
        InteriorRings => {
            let polygon = geometry(input(inputs, "geometry")?, Some(GeometryKind::Polygon))?;
            let rings = polygon["coordinates"].as_array().unwrap()[1..]
                .iter()
                .map(|ring| geometry(&json!({"type":"LineString", "coordinates":ring}), None))
                .collect::<Result<Vec<_>>>()?;
            Ok(vec![("rings", Value::Array(rings))])
        }
        NumInteriorRings => {
            let polygon = geometry(input(inputs, "geometry")?, Some(GeometryKind::Polygon))?;
            Ok(vec![(
                "count",
                json!(polygon["coordinates"].as_array().unwrap().len() - 1),
            )])
        }
        Boundary => geometry_result(boundary(input(inputs, "geometry")?)?),
        ToMulti => geometry_result(change_simple_multi(input(inputs, "geometry")?, true)?),
        ToSingle => geometry_result(change_simple_multi(input(inputs, "geometry")?, false)?),
    }
}

#[cfg(feature = "execute")]
async fn run_operation(operation: Operation, context: &mut ExecutionContext) -> Result<()> {
    let mut inputs = Map::new();
    for pin in definition(operation)
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input)
    {
        inputs.insert(
            pin.name.clone(),
            context.evaluate_pin::<Value>(&pin.name).await?,
        );
    }
    let outputs = super::cpu::run(move || execute(operation, &Value::Object(inputs))).await?;
    for (name, value) in outputs {
        context.set_pin_value(name, value).await?;
    }
    Ok(())
}

macro_rules! implement_node {
    ($name:ident, $operation:expr) => {
        #[async_trait]
        impl NodeLogic for $name {
            fn get_node(&self) -> Node {
                definition($operation)
            }

            #[cfg(feature = "execute")]
            async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
                run_operation($operation, context).await
            }

            #[cfg(not(feature = "execute"))]
            async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
                Err(flow_like_types::anyhow!(
                    "Geometry operations require the execute feature"
                ))
            }
        }
    };
}

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeLineStringNode;
implement_node!(GeometryMakeLineStringNode, Operation::MakeLineString);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakePolygonNode;
implement_node!(GeometryMakePolygonNode, Operation::MakePolygon);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeMultiPointNode;
implement_node!(GeometryMakeMultiPointNode, Operation::MakeMultiPoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeMultiLineStringNode;
implement_node!(
    GeometryMakeMultiLineStringNode,
    Operation::MakeMultiLineString
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeMultiPolygonNode;
implement_node!(GeometryMakeMultiPolygonNode, Operation::MakeMultiPolygon);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeGeometryCollectionNode;
implement_node!(
    GeometryMakeGeometryCollectionNode,
    Operation::MakeGeometryCollection
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeEnvelopeNode;
implement_node!(GeometryMakeEnvelopeNode, Operation::MakeEnvelope);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPartsNode;
implement_node!(GeometryPartsNode, Operation::Parts);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryNNode;
implement_node!(GeometryNNode, Operation::GeometryN);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryNumGeometriesNode;
implement_node!(GeometryNumGeometriesNode, Operation::NumGeometries);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPointsNode;
implement_node!(GeometryPointsNode, Operation::Points);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPointNNode;
implement_node!(GeometryPointNNode, Operation::PointN);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryStartPointNode;
implement_node!(GeometryStartPointNode, Operation::StartPoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryEndPointNode;
implement_node!(GeometryEndPointNode, Operation::EndPoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryExteriorRingNode;
implement_node!(GeometryExteriorRingNode, Operation::ExteriorRing);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryInteriorRingsNode;
implement_node!(GeometryInteriorRingsNode, Operation::InteriorRings);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryNumInteriorRingsNode;
implement_node!(GeometryNumInteriorRingsNode, Operation::NumInteriorRings);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryBoundaryNode;
implement_node!(GeometryBoundaryNode, Operation::Boundary);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToMultiNode;
implement_node!(GeometryToMultiNode, Operation::ToMulti);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToSingleNode;
implement_node!(GeometryToSingleNode, Operation::ToSingle);

#[cfg(test)]
mod definition_tests {
    use super::*;

    fn pin(node: &Node, name: &str) -> Pin {
        node.pins
            .values()
            .find(|pin| pin.name == name)
            .unwrap()
            .clone()
    }

    #[test]
    fn constructors_expose_typed_geometry_arrays() {
        let line = definition(Operation::MakeLineString);
        let points = pin(&line, "points");
        assert_eq!(points.data_type, VariableType::Geometry);
        assert_eq!(points.value_type, ValueType::Array);
        assert_eq!(points.schema.as_deref(), Some(marker(GeometryKind::Point)));
        let output = pin(&line, "geometry_out");
        assert_eq!(
            output.schema.as_deref(),
            Some(marker(GeometryKind::LineString))
        );

        let polygon = definition(Operation::MakePolygon);
        assert_eq!(pin(&polygon, "holes").value_type, ValueType::Array);
        assert_eq!(
            pin(&polygon, "geometry_out").schema.as_deref(),
            Some(marker(GeometryKind::Polygon))
        );
    }

    #[test]
    fn variable_cardinality_outputs_are_arrays() {
        for (operation, name, kind) in [
            (Operation::Parts, "parts", None),
            (Operation::Points, "points", Some(GeometryKind::Point)),
            (
                Operation::InteriorRings,
                "rings",
                Some(GeometryKind::LineString),
            ),
        ] {
            let output = pin(&definition(operation), name);
            assert_eq!(output.data_type, VariableType::Geometry);
            assert_eq!(output.value_type, ValueType::Array);
            assert_eq!(
                output.schema.as_deref(),
                kind.map(marker),
                "unexpected subtype for {name}"
            );
        }
    }
}

#[cfg(all(test, feature = "execute"))]
mod execution_tests {
    use super::*;

    fn point(x: f64, y: f64) -> Value {
        json!({"type":"Point", "coordinates":[x,y]})
    }

    fn line(coordinates: Value) -> Value {
        json!({"type":"LineString", "coordinates":coordinates})
    }

    fn result(operation: Operation, inputs: Value, name: &str) -> Value {
        execute(operation, &inputs)
            .unwrap()
            .into_iter()
            .find(|(output, _)| *output == name)
            .unwrap()
            .1
    }

    #[test]
    fn constructors_validate_subtypes_bounds_and_ring_shapes() {
        let made_line = result(
            Operation::MakeLineString,
            json!({"points":[point(0.0,0.0),point(2.0,1.0)]}),
            "geometry_out",
        );
        assert_eq!(made_line, line(json!([[0.0, 0.0], [2.0, 1.0]])));
        assert!(
            execute(
                Operation::MakeLineString,
                &json!({"points":[point(0.0,0.0)]})
            )
            .is_err()
        );
        assert!(
            execute(
                Operation::MakeMultiPoint,
                &json!({"points":[line(json!([[0,0],[1,1]]))]})
            )
            .is_err()
        );

        let polygon = result(
            Operation::MakePolygon,
            json!({
                "exterior":line(json!([[0,0],[4,0],[4,4],[0,4]])),
                "holes":[line(json!([[1,1],[1,2],[2,2],[2,1]]))]
            }),
            "geometry_out",
        );
        assert_eq!(polygon["coordinates"][0].as_array().unwrap().len(), 5);
        assert_eq!(polygon["coordinates"][1].as_array().unwrap().len(), 5);
        assert!(
            execute(
                Operation::MakePolygon,
                &json!({
                    "exterior":line(json!([[0,0],[2,2],[0,2],[2,0],[0,0]])),
                    "holes":[]
                })
            )
            .is_err()
        );

        let envelope = result(
            Operation::MakeEnvelope,
            json!({"west":-2,"south":3,"east":4,"north":5}),
            "geometry_out",
        );
        assert_eq!(envelope["type"], "Polygon");
        assert!(
            execute(
                Operation::MakeEnvelope,
                &json!({"west":4,"south":3,"east":-2,"north":5})
            )
            .is_err()
        );
        assert!(
            execute(
                Operation::MakeEnvelope,
                &json!({"west":-181,"south":3,"east":4,"north":5})
            )
            .is_err()
        );
    }

    #[test]
    fn multi_constructors_and_parts_round_trip() {
        let multi = result(
            Operation::MakeMultiLineString,
            json!({"lines":[
                line(json!([[0,0],[1,1]])),
                line(json!([[2,2],[3,3]]))
            ]}),
            "geometry_out",
        );
        let parts = result(Operation::Parts, json!({"geometry":multi}), "parts");
        assert_eq!(parts.as_array().unwrap().len(), 2);
        assert_eq!(parts[0]["type"], "LineString");
        assert_eq!(
            result(
                Operation::GeometryN,
                json!({"geometry":multi,"index":1}),
                "geometry_out"
            ),
            parts[1]
        );
        assert_eq!(
            result(Operation::NumGeometries, json!({"geometry":multi}), "count"),
            2
        );
        assert!(execute(Operation::GeometryN, &json!({"geometry":multi,"index":2})).is_err());
    }

    #[test]
    fn coordinate_and_ring_access_preserve_traversal_order() {
        let polygon = json!({
            "type":"Polygon",
            "coordinates":[
                [[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]],
                [[1.0,1.0],[1.0,2.0],[2.0,2.0],[2.0,1.0],[1.0,1.0]]
            ]
        });
        let points = result(Operation::Points, json!({"geometry":polygon}), "points");
        assert_eq!(points.as_array().unwrap().len(), 10);
        assert_eq!(points[0], point(0.0, 0.0));
        assert_eq!(points[4], point(0.0, 0.0));
        assert_eq!(
            result(
                Operation::PointN,
                json!({"geometry":polygon,"index":5}),
                "geometry_out"
            ),
            points[5]
        );
        assert_eq!(
            result(
                Operation::NumInteriorRings,
                json!({"geometry":polygon}),
                "count"
            ),
            1
        );
        assert_eq!(
            result(
                Operation::InteriorRings,
                json!({"geometry":polygon}),
                "rings"
            )[0]["type"],
            "LineString"
        );

        let open_line = line(json!([[2.0, 3.0], [4.0, 5.0], [6.0, 7.0]]));
        assert_eq!(
            result(
                Operation::StartPoint,
                json!({"geometry":open_line}),
                "geometry_out"
            ),
            point(2.0, 3.0)
        );
        assert_eq!(
            result(
                Operation::EndPoint,
                json!({"geometry":open_line}),
                "geometry_out"
            ),
            point(6.0, 7.0)
        );
    }

    #[test]
    fn boundary_uses_mod_two_line_endpoints_and_polygon_rings() {
        let lines = json!({
            "type":"MultiLineString",
            "coordinates":[[[0,0],[1,1]],[[1,1],[2,2]],[[5,5],[6,6],[5,5]]]
        });
        let boundary = result(
            Operation::Boundary,
            json!({"geometry":lines}),
            "geometry_out",
        );
        assert_eq!(
            boundary,
            json!({"type":"MultiPoint","coordinates":[[0,0],[2,2]]})
        );

        let polygon = json!({
            "type":"Polygon",
            "coordinates":[
                [[0,0],[4,0],[4,4],[0,4],[0,0]],
                [[1,1],[1,2],[2,2],[2,1],[1,1]]
            ]
        });
        let boundary = result(
            Operation::Boundary,
            json!({"geometry":polygon}),
            "geometry_out",
        );
        assert_eq!(boundary["type"], "MultiLineString");
        assert_eq!(boundary["coordinates"].as_array().unwrap().len(), 2);
        assert_eq!(
            result(
                Operation::Boundary,
                json!({"geometry":point(1.0,2.0)}),
                "geometry_out"
            ),
            json!({"type":"GeometryCollection","geometries":[]})
        );
    }

    #[test]
    fn collection_boundary_applies_global_rules_to_homogeneous_members() {
        let joined_lines = json!({
            "type":"GeometryCollection",
            "geometries":[
                line(json!([[0,0],[1,0]])),
                {
                    "type":"GeometryCollection",
                    "geometries":[line(json!([[1,0],[2,0]]))]
                }
            ]
        });
        assert_eq!(
            result(
                Operation::Boundary,
                json!({"geometry":joined_lines}),
                "geometry_out"
            ),
            json!({"type":"MultiPoint","coordinates":[[0,0],[2,0]]})
        );

        let point_collection = json!({
            "type":"GeometryCollection",
            "geometries":[point(1.0,2.0), {"type":"MultiPoint","coordinates":[]}]
        });
        assert_eq!(
            result(
                Operation::Boundary,
                json!({"geometry":point_collection}),
                "geometry_out"
            ),
            json!({"type":"GeometryCollection","geometries":[]})
        );

        let polygons = json!({
            "type":"GeometryCollection",
            "geometries":[
                {
                    "type":"Polygon",
                    "coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]
                },
                {
                    "type":"Polygon",
                    "coordinates":[[[2,0],[3,0],[3,1],[2,1],[2,0]]]
                }
            ]
        });
        let boundary = result(
            Operation::Boundary,
            json!({"geometry":polygons}),
            "geometry_out",
        );
        assert_eq!(boundary["type"], "MultiLineString");
        assert_eq!(boundary["coordinates"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn collection_boundary_ignores_points_and_rejects_mixed_positive_dimensions() {
        let points_and_line = json!({
            "type":"GeometryCollection",
            "geometries":[point(0.0,0.0), line(json!([[1,0],[2,0]]))]
        });
        assert_eq!(
            result(
                Operation::Boundary,
                json!({"geometry":points_and_line}),
                "geometry_out"
            ),
            json!({"type":"MultiPoint","coordinates":[[1,0],[2,0]]})
        );

        let mixed = json!({
            "type":"GeometryCollection",
            "geometries":[
                line(json!([[0,0],[1,0]])),
                {
                    "type":"Polygon",
                    "coordinates":[[[2,0],[3,0],[3,1],[2,1],[2,0]]]
                }
            ]
        });
        assert!(
            execute(Operation::Boundary, &json!({"geometry":mixed}))
                .unwrap_err()
                .to_string()
                .contains("both lineal and polygonal")
        );

        let shared_edge = json!({
            "type":"GeometryCollection",
            "geometries":[
                {
                    "type":"Polygon",
                    "coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]
                },
                {
                    "type":"Polygon",
                    "coordinates":[[[1,0],[2,0],[2,1],[1,1],[1,0]]]
                }
            ]
        });
        assert!(
            execute(Operation::Boundary, &json!({"geometry":shared_edge}))
                .unwrap_err()
                .to_string()
                .contains("valid MultiPolygon")
        );
    }

    #[test]
    fn single_multi_conversion_requires_unambiguous_unwrap() {
        let source = point(1.0, 2.0);
        let multi = result(
            Operation::ToMulti,
            json!({"geometry":source}),
            "geometry_out",
        );
        assert_eq!(
            multi,
            json!({"type":"MultiPoint","coordinates":[[1.0,2.0]]})
        );
        assert_eq!(
            result(
                Operation::ToSingle,
                json!({"geometry":multi}),
                "geometry_out"
            ),
            source
        );
        assert!(
            execute(
                Operation::ToSingle,
                &json!({"geometry":{"type":"MultiPoint","coordinates":[]}})
            )
            .is_err()
        );
        assert!(
            execute(
                Operation::ToSingle,
                &json!({"geometry":{"type":"MultiPoint","coordinates":[[0,0],[1,1]]}})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn parallel_geometry_array_validation_preserves_member_order() {
        const COUNT: usize = 16_384;
        let points = (0..COUNT)
            .map(|index| point(-170.0 + (index % 340) as f64, (index / 1_000) as f64))
            .collect::<Vec<_>>();
        let values = super::super::cpu::run(move || {
            execute(Operation::MakeMultiPoint, &json!({"points":points}))
        })
        .await
        .unwrap();
        let geometry = values
            .into_iter()
            .find(|(name, _)| *name == "geometry_out")
            .unwrap()
            .1;
        let coordinates = geometry["coordinates"].as_array().unwrap();
        assert_eq!(coordinates.len(), COUNT);
        assert_eq!(coordinates[0], json!([-170.0, 0.0]));
        assert_eq!(coordinates[COUNT - 1], json!([-107.0, 16.0]));
    }

    #[test]
    fn geometry_array_preflight_bounds_unvalidated_nesting() {
        let mut nested = point(0.0, 0.0);
        for _ in 0..=flow_like_types::geometry::MAX_GEOMETRY_DEPTH {
            nested = json!({"type":"GeometryCollection", "geometries":[nested]});
        }
        let error = geometry_array(&json!([nested]), None).unwrap_err();
        assert!(error.to_string().contains("depth or member limit"));
    }
}
