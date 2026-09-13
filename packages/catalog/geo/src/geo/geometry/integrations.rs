//! Geometry-native adapters for GeoJSON wrappers, encoded paths and spatial indexes.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    geometry::{GeometryKind, marker},
    json::json,
};

#[cfg(feature = "execute")]
use super::{add_estimated_geometry_positions, check_topology_validation, cpu};
#[cfg(feature = "execute")]
use flow_like_geometry::{from_geo, to_geo};
#[cfg(feature = "execute")]
use flow_like_types::{
    Result, Value, anyhow, bail,
    geometry::{MAX_GEOMETRY_POSITIONS, canonicalize_geometry},
    json::Map,
};
#[cfg(feature = "execute")]
use geo::CoordsIter;

#[derive(Clone, Copy, Debug)]
enum Operation {
    FeatureGeometry,
    MakeFeature,
    FeatureCollectionGeometries,
    MakeFeatureCollection,
    EncodePolyline,
    DecodePolyline,
    PointToH3Cell,
    H3CellBoundary,
    H3CellsToGeometry,
    PolygonToH3Cells,
    EncodeGeohash,
    DecodeGeohash,
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
        FeatureGeometry => (
            "geometry_feature_geometry",
            "featureGeometry",
            "GeoJSON Feature Geometry",
            "Extracts and validates the non-null Geometry and properties from a GeoJSON Feature.",
        ),
        MakeFeature => (
            "geometry_make_feature",
            "makeFeature",
            "Make GeoJSON Feature",
            "Wraps a Geometry and properties in a GeoJSON Feature. A non-empty id is included as the Feature id.",
        ),
        FeatureCollectionGeometries => (
            "geometry_feature_collection_geometries",
            "featureCollectionGeometries",
            "GeoJSON FeatureCollection Geometries",
            "Extracts validated geometries and properties from a GeoJSON FeatureCollection. Null Feature geometries are rejected because Geometry values cannot be null.",
        ),
        MakeFeatureCollection => (
            "geometry_make_feature_collection",
            "makeFeatureCollection",
            "Make GeoJSON FeatureCollection",
            "Creates a GeoJSON FeatureCollection from Geometry values and an optional matching array of property objects.",
        ),
        EncodePolyline => (
            "geometry_encode_polyline",
            "encodePolyline",
            "Encode Polyline",
            "Encodes a LineString with the Google encoded polyline algorithm. Precision is the number of decimal coordinate digits.",
        ),
        DecodePolyline => (
            "geometry_decode_polyline",
            "decodePolyline",
            "Decode Polyline",
            "Decodes a Google encoded polyline into a validated WGS 84 LineString.",
        ),
        PointToH3Cell => (
            "geometry_point_to_h3_cell",
            "pointToH3Cell",
            "Point to H3 Cell",
            "Converts a Geometry Point to an H3 cell index at resolution 0 through 15.",
        ),
        H3CellBoundary => (
            "geometry_h3_cell_boundary",
            "h3CellBoundary",
            "H3 Cell Boundary Geometry",
            "Returns an H3 cell boundary as a Polygon, or as a split MultiPolygon when the cell crosses the antimeridian.",
        ),
        H3CellsToGeometry => (
            "geometry_h3_cells_to_geometry",
            "h3CellsToGeometry",
            "H3 Cells to Geometry",
            "Dissolves unique H3 cells at one resolution into a MultiPolygon Geometry.",
        ),
        PolygonToH3Cells => (
            "geometry_polygon_to_h3_cells",
            "polygonToH3Cells",
            "Polygon to H3 Cells",
            "Covers a Polygon or MultiPolygon with H3 cells. Longitude edges use the Geometry contract's direct interpolation, so antimeridian regions must already be split. The containment mode controls whether centroids, complete boundaries or intersections qualify.",
        ),
        EncodeGeohash => (
            "geometry_encode_geohash",
            "encodeGeohash",
            "Point to Geohash",
            "Encodes a Geometry Point as a geohash with 1 through 12 characters.",
        ),
        DecodeGeohash => (
            "geometry_decode_geohash",
            "decodeGeohash",
            "Decode Geohash",
            "Decodes a geohash into its center Point and rectangular bounding Polygon.",
        ),
    };

    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");

    match operation {
        FeatureGeometry => {
            data_input(
                &mut node,
                "feature",
                "GeoJSON Feature object",
                VariableType::Struct,
            );
            geometry_output(&mut node, "geometry_out", "Extracted Geometry", None);
            data_output(
                &mut node,
                "properties",
                "Feature properties, or an empty object when omitted or null",
                VariableType::Struct,
            );
        }
        MakeFeature => {
            geometry_input(&mut node, "geometry", "Feature Geometry", None);
            data_input(
                &mut node,
                "properties",
                "Feature properties",
                VariableType::Struct,
            )
            .set_default_value(Some(json!({})));
            data_input(
                &mut node,
                "id",
                "Optional string Feature id",
                VariableType::String,
            )
            .set_default_value(Some(json!("")));
            data_output(
                &mut node,
                "feature",
                "GeoJSON Feature object",
                VariableType::Struct,
            );
        }
        FeatureCollectionGeometries => {
            data_input(
                &mut node,
                "feature_collection",
                "GeoJSON FeatureCollection object",
                VariableType::Struct,
            );
            geometry_output(
                &mut node,
                "geometry_out",
                "GeometryCollection containing each Feature Geometry",
                Some(GeometryKind::GeometryCollection),
            );
            data_output(
                &mut node,
                "properties",
                "Property objects in Feature order",
                VariableType::Struct,
            )
            .set_value_type(ValueType::Array);
        }
        MakeFeatureCollection => {
            geometry_input(&mut node, "geometries", "Feature geometries", None)
                .set_value_type(ValueType::Array)
                .set_default_value(Some(json!([])));
            data_input(
                &mut node,
                "properties",
                "Property objects, either empty or one per Geometry",
                VariableType::Struct,
            )
            .set_value_type(ValueType::Array)
            .set_default_value(Some(json!([])));
            data_output(
                &mut node,
                "feature_collection",
                "GeoJSON FeatureCollection object",
                VariableType::Struct,
            );
        }
        EncodePolyline => {
            geometry_input(
                &mut node,
                "geometry",
                "LineString to encode",
                Some(GeometryKind::LineString),
            );
            data_input(
                &mut node,
                "precision",
                "Decimal coordinate digits, from 0 through 10",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(5)));
            data_output(
                &mut node,
                "polyline",
                "Encoded polyline text",
                VariableType::String,
            );
        }
        DecodePolyline => {
            data_input(
                &mut node,
                "polyline",
                "Encoded polyline text",
                VariableType::String,
            );
            data_input(
                &mut node,
                "precision",
                "Decimal coordinate digits, from 0 through 10",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(5)));
            geometry_output(
                &mut node,
                "geometry_out",
                "Decoded LineString",
                Some(GeometryKind::LineString),
            );
        }
        PointToH3Cell => {
            geometry_input(
                &mut node,
                "geometry",
                "Point to index",
                Some(GeometryKind::Point),
            );
            data_input(
                &mut node,
                "resolution",
                "H3 resolution from 0 through 15",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(9)));
            data_output(&mut node, "cell", "H3 cell index", VariableType::String);
        }
        H3CellBoundary => {
            data_input(&mut node, "cell", "H3 cell index", VariableType::String);
            geometry_output(
                &mut node,
                "geometry_out",
                "Cell boundary Polygon or antimeridian-split MultiPolygon",
                None,
            );
        }
        H3CellsToGeometry => {
            data_input(
                &mut node,
                "cells",
                "Unique H3 cells at one resolution",
                VariableType::String,
            )
            .set_value_type(ValueType::Array)
            .set_default_value(Some(json!([])));
            geometry_output(
                &mut node,
                "geometry_out",
                "Dissolved MultiPolygon",
                Some(GeometryKind::MultiPolygon),
            );
        }
        PolygonToH3Cells => {
            geometry_input(
                &mut node,
                "geometry",
                "Polygon or MultiPolygon to cover",
                None,
            );
            data_input(
                &mut node,
                "resolution",
                "H3 resolution from 0 through 15",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(9)));
            data_input(
                &mut node,
                "containment",
                "One of centroid, contains-boundary, intersects-boundary or covers",
                VariableType::String,
            )
            .set_default_value(Some(json!("centroid")));
            data_input(
                &mut node,
                "max_cells",
                "Maximum cells to emit and basis for the preflight work budget, from 1 through 1000000",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(100_000)));
            data_output(&mut node, "cells", "H3 cell indexes", VariableType::String)
                .set_value_type(ValueType::Array);
        }
        EncodeGeohash => {
            geometry_input(
                &mut node,
                "geometry",
                "Point to encode",
                Some(GeometryKind::Point),
            );
            data_input(
                &mut node,
                "precision",
                "Geohash length from 1 through 12",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(9)));
            data_output(
                &mut node,
                "geohash",
                "Encoded geohash",
                VariableType::String,
            );
        }
        DecodeGeohash => {
            data_input(&mut node, "geohash", "Geohash text", VariableType::String);
            geometry_output(
                &mut node,
                "center",
                "Geohash cell center",
                Some(GeometryKind::Point),
            );
            geometry_output(
                &mut node,
                "bounds",
                "Geohash cell bounds",
                Some(GeometryKind::Polygon),
            );
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
        .ok_or_else(|| anyhow!("Missing input {name}"))
}

#[cfg(feature = "execute")]
fn integer(inputs: &Value, name: &str, minimum: i64, maximum: i64) -> Result<i64> {
    let value = input(inputs, name)?
        .as_i64()
        .ok_or_else(|| anyhow!("{name} must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be from {minimum} through {maximum}");
    }
    Ok(value)
}

#[cfg(feature = "execute")]
fn object<'a>(value: &'a Value, name: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| anyhow!("{name} must be an object"))
}

#[cfg(feature = "execute")]
fn canonical(value: &Value, kind: Option<GeometryKind>) -> Result<Value> {
    let value = canonicalize_geometry(value, kind)?;
    check_topology_validation(&to_geo(&value)?, "Invalid geometry")?;
    Ok(value)
}

#[cfg(feature = "execute")]
fn bounded_geometry_values_work(values: &[Value], name: &str) -> Result<usize> {
    if values.len() > MAX_GEOMETRY_POSITIONS {
        bail!("{name} cannot contain more than {MAX_GEOMETRY_POSITIONS} members");
    }
    let mut positions = 0usize;
    let mut members = 1usize;
    for geometry in values {
        add_estimated_geometry_positions(geometry, &mut positions, &mut members, name)?;
    }
    Ok(positions.saturating_add(values.len()))
}

#[cfg(feature = "execute")]
fn bounded_feature_work(features: &[Value]) -> Result<usize> {
    if features.len() > MAX_GEOMETRY_POSITIONS {
        bail!("FeatureCollection cannot contain more than {MAX_GEOMETRY_POSITIONS} features");
    }
    let mut positions = 0usize;
    let mut members = 1usize;
    for feature in features {
        if let Some(geometry) = feature.get("geometry") {
            add_estimated_geometry_positions(
                geometry,
                &mut positions,
                &mut members,
                "FeatureCollection geometries",
            )?;
        }
    }
    Ok(positions.saturating_add(features.len()))
}

#[cfg(feature = "execute")]
fn point_coordinates(value: &Value) -> Result<(f64, f64)> {
    let point = canonical(value, Some(GeometryKind::Point))?;
    let coordinates = point["coordinates"]
        .as_array()
        .ok_or_else(|| anyhow!("Point coordinates must be an array"))?;
    Ok((
        coordinates[0].as_f64().unwrap(),
        coordinates[1].as_f64().unwrap(),
    ))
}

#[cfg(feature = "execute")]
fn feature_parts(value: &Value, name: &str) -> Result<(Value, Value)> {
    let feature = object(value, name)?;
    if feature.get("type").and_then(Value::as_str) != Some("Feature") {
        bail!("{name} must have GeoJSON type Feature");
    }
    let geometry = feature
        .get("geometry")
        .ok_or_else(|| anyhow!("{name} requires a geometry member"))?;
    if geometry.is_null() {
        bail!("{name} has a null geometry, which cannot become a Geometry value");
    }
    let properties = match feature.get("properties") {
        None | Some(Value::Null) => json!({}),
        Some(value) => {
            object(value, "Feature properties")?;
            value.clone()
        }
    };
    Ok((canonical(geometry, None)?, properties))
}

#[cfg(feature = "execute")]
fn polyline_precision(inputs: &Value) -> Result<(u32, f64)> {
    let precision = integer(inputs, "precision", 0, 10)? as u32;
    Ok((precision, 10_f64.powi(precision as i32)))
}

#[cfg(feature = "execute")]
fn encode_polyline_value(value: i64, output: &mut String) -> Result<()> {
    let mut encoded = if value < 0 {
        (value.unsigned_abs() << 1).saturating_sub(1)
    } else {
        (value as u64) << 1
    };
    while encoded >= 0x20 {
        let code = ((encoded & 0x1f) | 0x20) + 63;
        output.push(char::from_u32(code as u32).ok_or_else(|| anyhow!("Invalid polyline"))?);
        encoded >>= 5;
    }
    output.push(char::from_u32((encoded + 63) as u32).ok_or_else(|| anyhow!("Invalid polyline"))?);
    Ok(())
}

#[cfg(feature = "execute")]
fn decode_polyline_value(bytes: &[u8], cursor: &mut usize) -> Result<i64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes
            .get(*cursor)
            .ok_or_else(|| anyhow!("Encoded polyline ended inside a coordinate"))?;
        *cursor += 1;
        if !(63..=126).contains(&byte) {
            bail!("Encoded polyline contains an invalid character");
        }
        let chunk = u64::from(byte - 63);
        if shift >= 64 || (chunk & 0x1f) > (u64::MAX >> shift) {
            bail!("Encoded polyline coordinate is too large");
        }
        result |= (chunk & 0x1f) << shift;
        if chunk < 0x20 {
            break;
        }
        shift += 5;
        if shift > 60 {
            bail!("Encoded polyline coordinate is too large");
        }
    }
    let magnitude = (result >> 1) as i64;
    Ok(if result & 1 == 1 {
        -magnitude - 1
    } else {
        magnitude
    })
}

#[cfg(feature = "execute")]
fn h3_resolution(inputs: &Value) -> Result<h3o::Resolution> {
    h3o::Resolution::try_from(integer(inputs, "resolution", 0, 15)? as u8)
        .map_err(|error| anyhow!("Invalid H3 resolution: {error}"))
}

#[cfg(feature = "execute")]
fn parse_cells(value: &Value) -> Result<Vec<h3o::CellIndex>> {
    use std::{collections::HashSet, str::FromStr};
    let values = value
        .as_array()
        .ok_or_else(|| anyhow!("cells must be an array"))?;
    if values.len() > 100_000 {
        bail!("cells cannot contain more than 100000 indexes");
    }
    let mut cells = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow!("cells[{index}] must be a string"))?;
        let cell = h3o::CellIndex::from_str(text)
            .map_err(|error| anyhow!("Invalid H3 cell at index {index}: {error}"))?;
        if seen.insert(cell) {
            cells.push(cell);
        }
    }
    Ok(cells)
}

#[cfg(feature = "execute")]
fn prepare_polygon_for_h3(polygon: geo::Polygon<f64>) -> geo::Polygon<f64> {
    fn subdivide(ring: geo::LineString<f64>) -> geo::LineString<f64> {
        const MAX_LONGITUDE_STEP: f64 = 90.0;

        let mut output = Vec::with_capacity(ring.0.len());
        if let Some(first) = ring.0.first().copied() {
            output.push(first);
        }
        for edge in ring.0.windows(2) {
            let start = edge[0];
            let end = edge[1];
            let pieces = ((end.x - start.x).abs() / MAX_LONGITUDE_STEP)
                .ceil()
                .max(1.0) as usize;
            for step in 1..=pieces {
                let ratio = step as f64 / pieces as f64;
                output.push(geo::Coord {
                    x: start.x + ratio * (end.x - start.x),
                    y: start.y + ratio * (end.y - start.y),
                });
            }
        }
        geo::LineString::new(output)
    }

    let (exterior, interiors) = polygon.into_inner();
    geo::Polygon::new(
        subdivide(exterior),
        interiors.into_iter().map(subdivide).collect(),
    )
}

#[cfg(feature = "execute")]
fn split_h3_polygon_at_antimeridian(polygon: geo::Polygon<f64>) -> Result<Vec<geo::Polygon<f64>>> {
    use geo::{BooleanOps, MapCoords};

    let crosses = polygon
        .exterior()
        .0
        .windows(2)
        .any(|edge| (edge[0].x - edge[1].x).abs() > 180.0);
    if !crosses {
        return Ok(vec![polygon]);
    }

    let shifted = polygon.map_coords(|coordinate| geo::Coord {
        x: if coordinate.x < 0.0 {
            coordinate.x + 360.0
        } else {
            coordinate.x
        },
        y: coordinate.y,
    });
    check_topology_validation(&shifted, "Invalid transmeridian H3 dissolve polygon")?;
    let western_window = geo::Rect::new(
        geo::Coord { x: 180.0, y: -90.0 },
        geo::Coord { x: 360.0, y: 90.0 },
    )
    .to_polygon();
    let eastern_window = geo::Rect::new(
        geo::Coord { x: 0.0, y: -90.0 },
        geo::Coord { x: 180.0, y: 90.0 },
    )
    .to_polygon();

    let mut parts = shifted
        .intersection(&western_window)
        .0
        .into_iter()
        .map(|part| {
            part.map_coords(|coordinate| geo::Coord {
                x: if (coordinate.x - 180.0).abs() <= 1e-6 {
                    -180.0
                } else {
                    (coordinate.x - 360.0).clamp(-180.0, 180.0)
                },
                y: coordinate.y.clamp(-90.0, 90.0),
            })
        })
        .collect::<Vec<_>>();
    parts.extend(
        shifted
            .intersection(&eastern_window)
            .0
            .into_iter()
            .map(|part| {
                part.map_coords(|coordinate| geo::Coord {
                    x: coordinate.x.clamp(0.0, 180.0),
                    y: coordinate.y.clamp(-90.0, 90.0),
                })
            }),
    );
    if parts.is_empty() {
        bail!("Transmeridian H3 dissolve produced no polygon parts");
    }
    Ok(parts)
}

#[cfg(feature = "execute")]
fn normalize_h3_dissolve(geometry: geo::MultiPolygon<f64>) -> Result<geo::MultiPolygon<f64>> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let work = geometry.coords_count();
    if work > MAX_GEOMETRY_POSITIONS {
        bail!("H3 dissolve output exceeds the Geometry position limit of {MAX_GEOMETRY_POSITIONS}");
    }
    let emitted_positions = AtomicUsize::new(0);
    let split = cpu::map_ordered(
        &geometry.0,
        work,
        |_, polygon| -> Result<Vec<geo::Polygon<f64>>> {
            let parts = split_h3_polygon_at_antimeridian(polygon.clone())?;
            let positions = parts.iter().fold(0usize, |positions, part| {
                positions.saturating_add(part.coords_count())
            });
            emitted_positions
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |emitted| {
                emitted.checked_add(positions).filter(|total| *total <= MAX_GEOMETRY_POSITIONS)
            })
            .map_err(|_| anyhow!(
                "Normalized H3 dissolve exceeds the Geometry position limit of {MAX_GEOMETRY_POSITIONS}"
            ))?;
            Ok(parts)
        },
    );
    let mut positions = 0usize;
    let mut polygons = Vec::with_capacity(geometry.0.len());
    for parts in split {
        for polygon in parts? {
            positions = positions.saturating_add(polygon.coords_count());
            if positions > MAX_GEOMETRY_POSITIONS {
                bail!(
                    "Normalized H3 dissolve exceeds the Geometry position limit of {MAX_GEOMETRY_POSITIONS}"
                );
            }
            polygons.push(polygon);
        }
    }
    Ok(geo::MultiPolygon::new(polygons))
}

#[cfg(feature = "execute")]
fn execute(operation: Operation, inputs: &Value) -> Result<Vec<(&'static str, Value)>> {
    use Operation::*;
    match operation {
        FeatureGeometry => {
            let (geometry, properties) = feature_parts(input(inputs, "feature")?, "feature")?;
            Ok(vec![("geometry_out", geometry), ("properties", properties)])
        }
        MakeFeature => {
            let geometry = canonical(input(inputs, "geometry")?, None)?;
            let properties = input(inputs, "properties")?;
            object(properties, "properties")?;
            let id = input(inputs, "id")?
                .as_str()
                .ok_or_else(|| anyhow!("id must be a string"))?;
            let mut feature = json!({
                "type":"Feature",
                "geometry":geometry,
                "properties":properties
            });
            if !id.is_empty() {
                feature
                    .as_object_mut()
                    .unwrap()
                    .insert("id".into(), json!(id));
            }
            Ok(vec![("feature", feature)])
        }
        FeatureCollectionGeometries => {
            let collection = object(input(inputs, "feature_collection")?, "feature_collection")?;
            if collection.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
                bail!("feature_collection must have GeoJSON type FeatureCollection");
            }
            let features = collection
                .get("features")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("FeatureCollection requires a features array"))?;
            let work = bounded_feature_work(features)?;
            let parts = cpu::map_ordered(features, work, |index, feature| {
                feature_parts(feature, &format!("features[{index}]"))
            });
            let mut geometries = Vec::with_capacity(features.len());
            let mut properties = Vec::with_capacity(features.len());
            for part in parts {
                let (geometry, feature_properties) = part?;
                geometries.push(geometry);
                properties.push(feature_properties);
            }
            let geometry = canonical(
                &json!({"type":"GeometryCollection", "geometries":geometries}),
                Some(GeometryKind::GeometryCollection),
            )?;
            Ok(vec![
                ("geometry_out", geometry),
                ("properties", Value::Array(properties)),
            ])
        }
        MakeFeatureCollection => {
            let geometries = input(inputs, "geometries")?
                .as_array()
                .ok_or_else(|| anyhow!("geometries must be an array"))?;
            let properties = input(inputs, "properties")?
                .as_array()
                .ok_or_else(|| anyhow!("properties must be an array"))?;
            if !properties.is_empty() && properties.len() != geometries.len() {
                bail!(
                    "properties must be empty or contain one object per Geometry: received {} properties for {} geometries",
                    properties.len(),
                    geometries.len()
                );
            }
            let work = bounded_geometry_values_work(geometries, "geometries")?;
            let features = cpu::map_ordered(geometries, work, |index, geometry| -> Result<Value> {
                let geometry = canonical(geometry, None)?;
                let properties = properties.get(index).cloned().unwrap_or_else(|| json!({}));
                object(&properties, &format!("properties[{index}]"))?;
                Ok(json!({
                    "type":"Feature",
                    "geometry":geometry,
                    "properties":properties
                }))
            })
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
            Ok(vec![(
                "feature_collection",
                json!({"type":"FeatureCollection", "features":features}),
            )])
        }
        EncodePolyline => {
            let geometry = canonical(input(inputs, "geometry")?, Some(GeometryKind::LineString))?;
            let (_, factor) = polyline_precision(inputs)?;
            let coordinates = geometry["coordinates"].as_array().unwrap();
            let mut previous_latitude = 0i64;
            let mut previous_longitude = 0i64;
            let mut polyline = String::with_capacity(coordinates.len() * 8);
            for position in coordinates {
                let position = position.as_array().unwrap();
                let longitude = (position[0].as_f64().unwrap() * factor).round() as i64;
                let latitude = (position[1].as_f64().unwrap() * factor).round() as i64;
                encode_polyline_value(latitude - previous_latitude, &mut polyline)?;
                encode_polyline_value(longitude - previous_longitude, &mut polyline)?;
                previous_latitude = latitude;
                previous_longitude = longitude;
            }
            Ok(vec![("polyline", json!(polyline))])
        }
        DecodePolyline => {
            let polyline = input(inputs, "polyline")?
                .as_str()
                .ok_or_else(|| anyhow!("polyline must be a string"))?;
            let (_, factor) = polyline_precision(inputs)?;
            let bytes = polyline.as_bytes();
            let mut cursor = 0usize;
            let mut latitude = 0i64;
            let mut longitude = 0i64;
            let mut coordinates = Vec::new();
            while cursor < bytes.len() {
                latitude = latitude
                    .checked_add(decode_polyline_value(bytes, &mut cursor)?)
                    .ok_or_else(|| anyhow!("Decoded latitude overflowed"))?;
                longitude = longitude
                    .checked_add(decode_polyline_value(bytes, &mut cursor)?)
                    .ok_or_else(|| anyhow!("Decoded longitude overflowed"))?;
                coordinates.push(json!([longitude as f64 / factor, latitude as f64 / factor]));
                if coordinates.len() > 100_000 {
                    bail!("Encoded polyline contains more than 100000 positions");
                }
            }
            let geometry = canonical(
                &json!({"type":"LineString", "coordinates":coordinates}),
                Some(GeometryKind::LineString),
            )?;
            Ok(vec![("geometry_out", geometry)])
        }
        PointToH3Cell => {
            let (longitude, latitude) = point_coordinates(input(inputs, "geometry")?)?;
            let coordinate = h3o::LatLng::new(latitude, longitude)
                .map_err(|error| anyhow!("Invalid Point for H3: {error}"))?;
            let cell = coordinate.to_cell(h3_resolution(inputs)?);
            Ok(vec![("cell", json!(cell.to_string()))])
        }
        H3CellBoundary => {
            use std::str::FromStr;
            let cell = input(inputs, "cell")?
                .as_str()
                .ok_or_else(|| anyhow!("cell must be a string"))?;
            let cell = h3o::CellIndex::from_str(cell)
                .map_err(|error| anyhow!("Invalid H3 cell: {error}"))?;
            // h3o's geometry conversion splits transmeridian cells at +/-180.
            // Building a ring directly from `CellIndex::boundary` would instead
            // create a nearly world-spanning planar edge between the two sides.
            let mut polygons = geo::MultiPolygon::from(cell);
            let geometry = if polygons.0.len() == 1 {
                geo::Geometry::Polygon(polygons.0.remove(0))
            } else {
                geo::Geometry::MultiPolygon(polygons)
            };
            let geometry = canonical(&from_geo(&geometry)?, None)?;
            Ok(vec![("geometry_out", geometry)])
        }
        H3CellsToGeometry => {
            use h3o::geom::SolventBuilder;
            let cells = parse_cells(input(inputs, "cells")?)?;
            if let Some(resolution) = cells.first().map(|cell| cell.resolution())
                && cells.iter().any(|cell| cell.resolution() != resolution)
            {
                bail!("All H3 cells must use the same resolution");
            }
            let geometry = SolventBuilder::new()
                .build()
                .dissolve(cells)
                .map_err(|error| anyhow!("Failed to dissolve H3 cells: {error}"))?;
            let geometry = normalize_h3_dissolve(geometry)?;
            let geometry = canonical(
                &from_geo(&geo::Geometry::MultiPolygon(geometry))?,
                Some(GeometryKind::MultiPolygon),
            )?;
            Ok(vec![("geometry_out", geometry)])
        }
        PolygonToH3Cells => {
            use geo::Geometry;
            use h3o::geom::{ContainmentMode, TilerBuilder};
            use std::sync::atomic::{AtomicUsize, Ordering};
            let geometry = to_geo(&canonical(input(inputs, "geometry")?, None)?)?;
            let containment = match input(inputs, "containment")?
                .as_str()
                .ok_or_else(|| anyhow!("containment must be a string"))?
            {
                "centroid" => ContainmentMode::ContainsCentroid,
                "contains-boundary" => ContainmentMode::ContainsBoundary,
                "intersects-boundary" => ContainmentMode::IntersectsBoundary,
                "covers" => ContainmentMode::Covers,
                value => bail!(
                    "Unknown containment mode {value}; expected centroid, contains-boundary, intersects-boundary or covers"
                ),
            };
            let resolution = h3_resolution(inputs)?;
            let maximum = integer(inputs, "max_cells", 1, 1_000_000)? as usize;
            let (polygons, geometry_label) = match geometry {
                Geometry::Polygon(polygon) => (vec![polygon], "Polygon"),
                Geometry::MultiPolygon(polygons) => (polygons.0, "MultiPolygon"),
                _ => bail!("H3 coverage requires Polygon or MultiPolygon"),
            };
            if polygons.is_empty() {
                return Ok(vec![("cells", json!([]))]);
            }
            let build_tiler = || {
                TilerBuilder::new(resolution)
                    .containment_mode(containment)
                    // Geometry longitude edges interpolate directly. Antimeridian
                    // regions must already be split before H3 coverage.
                    .disable_transmeridian_heuristic()
                    .build()
            };
            let tilers = if matches!(containment, ContainmentMode::Covers) {
                // Covers falls back to the combined geometry's centroid if its
                // outline search returns no cells. Preserve that collection behavior.
                let mut tiler = build_tiler();
                tiler
                    .add_batch(polygons.into_iter().map(prepare_polygon_for_h3))
                    .map_err(|error| {
                        anyhow!("Invalid {geometry_label} for H3 coverage: {error}")
                    })?;
                vec![tiler]
            } else {
                polygons
                    .into_iter()
                    .map(|polygon| {
                        let mut tiler = build_tiler();
                        tiler
                            .add(prepare_polygon_for_h3(polygon))
                            .map_err(|error| {
                                anyhow!("Invalid {geometry_label} for H3 coverage: {error}")
                            })?;
                        Ok(tiler)
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            // h3o constructs the complete outline and seed sets before its
            // coverage iterator can honor `take`. Keep that eager phase tied
            // to the caller's output budget, with modest estimator slack for
            // tiny polygons and boundary tracing.
            let estimated_work = tilers.iter().fold(0usize, |work, tiler| {
                work.saturating_add(tiler.coverage_size_hint())
            });
            let preflight_limit = maximum.saturating_mul(2).saturating_add(64);
            if estimated_work > preflight_limit {
                bail!(
                    "Estimated H3 coverage work ({estimated_work} cells) exceeds the preflight limit ({preflight_limit}) derived from max_cells ({maximum})"
                );
            }
            // Bound retained cell indexes across every worker, including duplicates
            // near neighboring polygon boundaries. Check max_cells after global dedup.
            let emitted_cells = AtomicUsize::new(0);
            let coverage = cpu::map_owned(tilers, estimated_work.saturating_mul(7), |_, tiler| {
                let mut cells = Vec::new();
                for cell in tiler.into_coverage().take(maximum + 1) {
                    emitted_cells
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |emitted| {
                            emitted.checked_add(1).filter(|total| *total <= preflight_limit)
                        })
                        .map_err(|_| anyhow!(
                            "H3 coverage exceeds the intermediate work limit ({preflight_limit}) derived from max_cells ({maximum})"
                        ))?;
                    cells.push(cell);
                }
                if cells.len() > maximum {
                    bail!("H3 coverage exceeds max_cells ({maximum})");
                }
                Ok(cells)
            });
            let mut cells = coverage
                .into_iter()
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            cells.sort_unstable();
            cells.dedup();
            if cells.len() > maximum {
                bail!("H3 coverage exceeds max_cells ({maximum})");
            }
            let cells: Vec<_> = cells.into_iter().map(|cell| cell.to_string()).collect();
            Ok(vec![("cells", json!(cells))])
        }
        EncodeGeohash => {
            let (longitude, latitude) = point_coordinates(input(inputs, "geometry")?)?;
            let precision = integer(inputs, "precision", 1, 12)? as usize;
            let geohash = geohash::encode(
                geohash::Coord {
                    x: longitude,
                    y: latitude,
                },
                precision,
            )
            .map_err(|error| anyhow!("Failed to encode geohash: {error}"))?;
            Ok(vec![("geohash", json!(geohash))])
        }
        DecodeGeohash => {
            let hash = input(inputs, "geohash")?
                .as_str()
                .ok_or_else(|| anyhow!("geohash must be a string"))?;
            if !(1..=12).contains(&hash.len()) {
                bail!("geohash must contain 1 through 12 ASCII characters");
            }
            let (center, _, _) = geohash::decode(hash)
                .map_err(|error| anyhow!("Failed to decode geohash: {error}"))?;
            let bounds = geohash::decode_bbox(hash)
                .map_err(|error| anyhow!("Failed to decode geohash bounds: {error}"))?;
            let west = bounds.min().x;
            let south = bounds.min().y;
            let east = bounds.max().x;
            let north = bounds.max().y;
            let center = canonical(
                &json!({"type":"Point", "coordinates":[center.x,center.y]}),
                Some(GeometryKind::Point),
            )?;
            let bounds = canonical(
                &json!({
                    "type":"Polygon",
                    "coordinates":[[
                        [west,south], [east,south], [east,north], [west,north], [west,south]
                    ]]
                }),
                Some(GeometryKind::Polygon),
            )?;
            Ok(vec![("center", center), ("bounds", bounds)])
        }
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
        let value = context.evaluate_pin::<Value>(&pin.name).await?;
        inputs.insert(pin.name.clone(), value);
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
pub struct GeometryFeatureGeometryNode;
implement_node!(GeometryFeatureGeometryNode, Operation::FeatureGeometry);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeFeatureNode;
implement_node!(GeometryMakeFeatureNode, Operation::MakeFeature);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFeatureCollectionGeometriesNode;
implement_node!(
    GeometryFeatureCollectionGeometriesNode,
    Operation::FeatureCollectionGeometries
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMakeFeatureCollectionNode;
implement_node!(
    GeometryMakeFeatureCollectionNode,
    Operation::MakeFeatureCollection
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryEncodePolylineNode;
implement_node!(GeometryEncodePolylineNode, Operation::EncodePolyline);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDecodePolylineNode;
implement_node!(GeometryDecodePolylineNode, Operation::DecodePolyline);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPointToH3CellNode;
implement_node!(GeometryPointToH3CellNode, Operation::PointToH3Cell);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryH3CellBoundaryNode;
implement_node!(GeometryH3CellBoundaryNode, Operation::H3CellBoundary);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryH3CellsToGeometryNode;
implement_node!(GeometryH3CellsToGeometryNode, Operation::H3CellsToGeometry);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPolygonToH3CellsNode;
implement_node!(GeometryPolygonToH3CellsNode, Operation::PolygonToH3Cells);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryEncodeGeohashNode;
implement_node!(GeometryEncodeGeohashNode, Operation::EncodeGeohash);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDecodeGeohashNode;
implement_node!(GeometryDecodeGeohashNode, Operation::DecodeGeohash);

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
    fn geometry_integrations_keep_geometry_pins_typed() {
        let point_to_h3 = definition(Operation::PointToH3Cell);
        assert_eq!(
            pin(&point_to_h3, "geometry").schema.as_deref(),
            Some(marker(GeometryKind::Point))
        );
        let feature_collection = definition(Operation::FeatureCollectionGeometries);
        assert_eq!(
            pin(&feature_collection, "geometry_out").schema.as_deref(),
            Some(marker(GeometryKind::GeometryCollection))
        );
        assert_eq!(
            pin(&definition(Operation::H3CellBoundary), "geometry_out").schema,
            None
        );
    }

    #[test]
    fn array_contracts_are_explicit() {
        assert_eq!(
            pin(&definition(Operation::PolygonToH3Cells), "cells").value_type,
            ValueType::Array
        );
        assert_eq!(
            pin(&definition(Operation::MakeFeatureCollection), "geometries").value_type,
            ValueType::Array
        );
    }
}

#[cfg(all(test, feature = "execute"))]
mod execution_tests {
    use super::*;
    use geo::Validation;

    fn output(operation: Operation, inputs: Value, name: &str) -> Value {
        execute(operation, &inputs)
            .unwrap()
            .into_iter()
            .find(|(pin, _)| *pin == name)
            .unwrap()
            .1
    }

    #[test]
    fn encoded_polyline_matches_reference_example() {
        let line = json!({
            "type":"LineString",
            "coordinates":[[-120.2,38.5],[-120.95,40.7],[-126.453,43.252]]
        });
        let encoded = output(
            Operation::EncodePolyline,
            json!({"geometry":line, "precision":5}),
            "polyline",
        );
        assert_eq!(encoded, json!("_p~iF~ps|U_ulLnnqC_mqNvxq`@"));
        let decoded = output(
            Operation::DecodePolyline,
            json!({"polyline":encoded, "precision":5}),
            "geometry_out",
        );
        assert_eq!(decoded["type"], "LineString");
        assert_eq!(decoded["coordinates"][2], json!([-126.453, 43.252]));
    }

    #[test]
    fn feature_round_trip_keeps_geometry_and_properties() {
        let feature = json!({
            "type":"Feature",
            "id":"place-1",
            "geometry":{"type":"Point", "coordinates":[13.4,52.5]},
            "properties":{"name":"Berlin"}
        });
        let values = execute(Operation::FeatureGeometry, &json!({"feature":feature})).unwrap();
        assert_eq!(values[0].1["coordinates"], json!([13.4, 52.5]));
        assert_eq!(values[1].1["name"], "Berlin");
    }

    #[tokio::test]
    async fn parallel_feature_collection_maps_preserve_feature_order() {
        const FEATURES: usize = 4;
        const POSITIONS: usize = 4_096;
        let features = (0..FEATURES)
            .map(|feature_index| {
                let coordinates = (0..POSITIONS)
                    .map(|position| json!([-170.0 + (position % 340) as f64, feature_index as f64]))
                    .collect::<Vec<_>>();
                json!({
                    "type":"Feature",
                    "geometry":{"type":"LineString", "coordinates":coordinates},
                    "properties":{"index":feature_index}
                })
            })
            .collect::<Vec<_>>();
        let rebuilt = super::super::cpu::run(move || -> Result<Value> {
            let extracted = execute(
                Operation::FeatureCollectionGeometries,
                &json!({
                    "feature_collection":{"type":"FeatureCollection", "features":features}
                }),
            )?;
            let collection = extracted
                .iter()
                .find(|(name, _)| *name == "geometry_out")
                .unwrap()
                .1
                .clone();
            let properties = extracted
                .iter()
                .find(|(name, _)| *name == "properties")
                .unwrap()
                .1
                .clone();
            let made = execute(
                Operation::MakeFeatureCollection,
                &json!({
                    "geometries":collection["geometries"],
                    "properties":properties
                }),
            )?;
            Ok(made
                .into_iter()
                .find(|(name, _)| *name == "feature_collection")
                .unwrap()
                .1)
        })
        .await
        .unwrap();
        let features = rebuilt["features"].as_array().unwrap();
        assert_eq!(features.len(), FEATURES);
        for (index, feature) in features.iter().enumerate() {
            assert_eq!(feature["properties"]["index"], index);
            assert_eq!(feature["geometry"]["coordinates"][0][1], index as f64);
        }
    }

    #[test]
    fn geohash_decode_returns_geometry_values() {
        let values = execute(Operation::DecodeGeohash, &json!({"geohash":"u33dc1"})).unwrap();
        assert_eq!(values[0].1["type"], "Point");
        assert_eq!(values[1].1["type"], "Polygon");
    }

    #[test]
    fn h3_boundary_splits_transmeridian_cells() {
        let boundary = output(
            Operation::H3CellBoundary,
            json!({"cell":"840d9edffffffff"}),
            "geometry_out",
        );
        assert_eq!(boundary["type"], "MultiPolygon");
        assert_eq!(boundary["coordinates"].as_array().unwrap().len(), 2);
        assert!(to_geo(&boundary).unwrap().is_valid());

        for polygon in boundary["coordinates"].as_array().unwrap() {
            let ring = polygon[0].as_array().unwrap();
            assert!(ring.windows(2).all(|edge| {
                let a = edge[0][0].as_f64().unwrap();
                let b = edge[1][0].as_f64().unwrap();
                (a - b).abs() <= 180.0
            }));
        }
    }

    #[test]
    fn h3_dissolve_splits_transmeridian_outlines() {
        let geometry = output(
            Operation::H3CellsToGeometry,
            json!({"cells":["840d9edffffffff"]}),
            "geometry_out",
        );
        assert_eq!(geometry["type"], "MultiPolygon");
        assert_eq!(geometry["coordinates"].as_array().unwrap().len(), 2);
        for polygon in geometry["coordinates"].as_array().unwrap() {
            let ring = polygon[0].as_array().unwrap();
            assert!(ring.windows(2).all(|edge| {
                let a = edge[0][0].as_f64().unwrap();
                let b = edge[1][0].as_f64().unwrap();
                (a - b).abs() <= 180.0
            }));
        }
    }

    #[test]
    fn h3_coverage_honors_direct_longitude_edges() {
        let equator_cell = output(
            Operation::PointToH3Cell,
            json!({
                "geometry":{"type":"Point", "coordinates":[0.0,0.0]},
                "resolution":2
            }),
            "cell",
        );
        let cells = output(
            Operation::PolygonToH3Cells,
            json!({
                "geometry":{"type":"Polygon", "coordinates":[[
                    [-170.0,-10.0], [170.0,-10.0], [170.0,10.0],
                    [-170.0,10.0], [-170.0,-10.0]
                ]]},
                "resolution":2,
                "containment":"centroid",
                "max_cells":10_000
            }),
            "cells",
        );
        assert!(cells.as_array().unwrap().contains(&equator_cell));
    }

    #[test]
    fn h3_coverage_rejects_excessive_estimated_work_before_tiling() {
        let error = execute(
            Operation::PolygonToH3Cells,
            &json!({
                "geometry":{"type":"Polygon", "coordinates":[[
                    [-179.0,-80.0], [179.0,-80.0], [179.0,80.0],
                    [-179.0,80.0], [-179.0,-80.0]
                ]]},
                "resolution":15,
                "containment":"centroid",
                "max_cells":10
            }),
        )
        .expect_err("large coverage must fail its preflight");
        assert!(error.to_string().contains("preflight limit"), "{error}");
    }

    #[test]
    fn h3_coverage_returns_empty_for_an_empty_multipolygon_in_covers_mode() {
        let cells = output(
            Operation::PolygonToH3Cells,
            json!({
                "geometry":{"type":"MultiPolygon", "coordinates":[]},
                "resolution":15,
                "containment":"covers",
                "max_cells":1
            }),
            "cells",
        );
        assert_eq!(cells, json!([]));
    }

    #[tokio::test]
    async fn parallel_h3_coverage_matches_combined_tiling_and_sorted_output() {
        use h3o::geom::{ContainmentMode, TilerBuilder};

        let polygons: Vec<_> = [(-40.0, -20.0), (-10.0, 0.0), (20.0, 20.0), (60.0, 40.0)]
            .into_iter()
            .map(|(x, y)| {
                geo::Polygon::new(
                    geo::LineString::from(vec![
                        (x, y),
                        (x + 10.0, y),
                        (x + 10.0, y + 10.0),
                        (x, y + 10.0),
                        (x, y),
                    ]),
                    vec![],
                )
            })
            .collect();
        let geometry = from_geo(&geo::Geometry::MultiPolygon(geo::MultiPolygon::new(
            polygons.clone(),
        )))
        .unwrap();
        for (mode, containment) in [
            (ContainmentMode::ContainsCentroid, "centroid"),
            (ContainmentMode::ContainsBoundary, "contains-boundary"),
            (ContainmentMode::IntersectsBoundary, "intersects-boundary"),
            (ContainmentMode::Covers, "covers"),
        ] {
            let mut combined = TilerBuilder::new(h3o::Resolution::Four)
                .containment_mode(mode)
                .disable_transmeridian_heuristic()
                .build();
            combined.add_batch(polygons.clone()).unwrap();
            assert!(combined.coverage_size_hint().saturating_mul(7) >= 16_384);
            let mut expected: Vec<_> = combined
                .into_coverage()
                .map(|cell| cell.to_string())
                .collect();
            expected.sort_unstable();
            expected.dedup();
            let inputs = json!({
                "geometry":geometry, "resolution":4, "containment":containment, "max_cells":100_000
            });
            let serial = execute(Operation::PolygonToH3Cells, &inputs).unwrap();
            assert_eq!(serial[0].1, json!(expected), "{containment}");
            let parallel = cpu::run(move || execute(Operation::PolygonToH3Cells, &inputs))
                .await
                .unwrap();
            assert_eq!(parallel, serial, "{containment}");
        }
    }

    #[tokio::test]
    async fn parallel_h3_normalization_preserves_polygon_order_and_bounds_output() {
        let polygons = (0..64)
            .map(|index| {
                let mut coordinates: Vec<_> = (0..256)
                    .map(|vertex| {
                        let angle = std::f64::consts::TAU * vertex as f64 / 256.0;
                        let longitude = 180.0 + angle.cos();
                        geo::Coord {
                            x: if longitude > 180.0 {
                                longitude - 360.0
                            } else {
                                longitude
                            },
                            y: -80.0 + index as f64 * 2.5 + angle.sin(),
                        }
                    })
                    .collect();
                coordinates.push(coordinates[0]);
                geo::Polygon::new(geo::LineString::new(coordinates), vec![])
            })
            .collect();
        let geometry = geo::MultiPolygon::new(polygons);
        assert!(geometry.coords_count() >= 16_384);
        let serial = normalize_h3_dissolve(geometry.clone()).unwrap();
        let parallel = cpu::run(move || normalize_h3_dissolve(geometry))
            .await
            .unwrap();
        assert_eq!(parallel, serial);
        assert_eq!(parallel.0.len(), 128);

        let crossing = geo::Polygon::new(
            geo::LineString::from(vec![
                (179.0, 0.0),
                (-179.0, 0.0),
                (-179.0, 1.0),
                (179.0, 1.0),
                (179.0, 0.0),
            ]),
            vec![],
        );
        let geometry = geo::MultiPolygon::new(vec![crossing; MAX_GEOMETRY_POSITIONS / 5]);
        let serial_error = normalize_h3_dissolve(geometry.clone())
            .unwrap_err()
            .to_string();
        let parallel_error = cpu::run(move || normalize_h3_dissolve(geometry))
            .await
            .unwrap_err()
            .to_string();
        assert!(serial_error.contains("position limit"));
        assert_eq!(parallel_error, serial_error);
    }

    #[tokio::test]
    async fn h3_multipolygon_coverage_preflights_the_aggregate_estimate() {
        use h3o::geom::TilerBuilder;

        let geometry = json!({"type":"MultiPolygon", "coordinates":[
                [[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0],[0.0,0.0]]],
                [[[20.0,20.0],[21.0,20.0],[21.0,21.0],[20.0,21.0],[20.0,20.0]]]
        ]});
        let geo::Geometry::MultiPolygon(polygons) = to_geo(&geometry).unwrap() else {
            unreachable!()
        };
        let estimates: Vec<_> = polygons
            .0
            .into_iter()
            .map(|polygon| {
                let mut tiler = TilerBuilder::new(h3o::Resolution::Six)
                    .disable_transmeridian_heuristic()
                    .build();
                tiler.add(polygon).unwrap();
                tiler.coverage_size_hint()
            })
            .collect();
        let maximum = estimates.iter().copied().max().unwrap().div_ceil(2);
        let limit = maximum * 2 + 64;
        assert!(estimates.iter().all(|estimate| *estimate <= limit));
        assert!(estimates.iter().sum::<usize>() > limit);
        let inputs = json!({
            "geometry":geometry, "resolution":6, "containment":"centroid", "max_cells":maximum
        });
        let serial = execute(Operation::PolygonToH3Cells, &inputs)
            .unwrap_err()
            .to_string();
        let parallel = cpu::run(move || execute(Operation::PolygonToH3Cells, &inputs))
            .await
            .unwrap_err()
            .to_string();
        assert!(serial.contains("preflight limit"));
        assert_eq!(parallel, serial);
    }

    #[tokio::test]
    async fn h3_collection_covers_preserves_disconnected_tiny_regions() {
        use h3o::geom::{ContainmentMode, TilerBuilder};

        let polygons: Vec<_> = [(1.0, 1.0), (20.0, 20.0)]
            .into_iter()
            .map(|(x, y)| {
                geo::Polygon::new(
                    geo::LineString::from(vec![
                        (x, y),
                        (x + 0.000001, y),
                        (x + 0.000001, y + 0.000001),
                        (x, y + 0.000001),
                        (x, y),
                    ]),
                    vec![],
                )
            })
            .collect();
        let mut combined = TilerBuilder::new(h3o::Resolution::One)
            .containment_mode(ContainmentMode::Covers)
            .disable_transmeridian_heuristic()
            .build();
        combined.add_batch(polygons.clone()).unwrap();
        let mut expected: Vec<_> = combined
            .into_coverage()
            .map(|cell| cell.to_string())
            .collect();
        expected.sort_unstable();
        let mut containing_cells: Vec<_> = [(1.0, 1.0), (20.0, 20.0)]
            .into_iter()
            .map(|(longitude, latitude)| {
                h3o::LatLng::new(latitude + 0.0000005, longitude + 0.0000005)
                    .unwrap()
                    .to_cell(h3o::Resolution::One)
                    .to_string()
            })
            .collect();
        containing_cells.sort_unstable();
        assert_ne!(containing_cells[0], containing_cells[1]);
        assert_eq!(expected, containing_cells);
        let geometry = from_geo(&geo::Geometry::MultiPolygon(geo::MultiPolygon::new(
            polygons,
        )))
        .unwrap();
        let mut inputs =
            json!({"geometry":geometry, "resolution":1, "containment":"covers", "max_cells":1});
        let error = execute(Operation::PolygonToH3Cells, &inputs).unwrap_err();
        assert!(error.to_string().contains("exceeds max_cells (1)"));
        inputs["max_cells"] = json!(2);
        let actual = cpu::run(move || execute(Operation::PolygonToH3Cells, &inputs))
            .await
            .unwrap();
        assert_eq!(actual[0].1, json!(expected));
    }

    #[tokio::test]
    async fn h3_collection_coverage_deduplicates_shared_boundary_cells_before_output_limit() {
        use h3o::geom::{ContainmentMode, TilerBuilder};

        let cell = h3o::LatLng::new(0.0, 0.0)
            .unwrap()
            .to_cell(h3o::Resolution::Three);
        let boundary = cell.boundary();
        let polygons: Vec<_> = [boundary[0], boundary[2]]
            .into_iter()
            .map(|point| {
                let (x, y) = (point.lng(), point.lat());
                geo::Polygon::new(
                    geo::LineString::from(vec![
                        (x - 0.001, y - 0.001),
                        (x + 0.001, y - 0.001),
                        (x + 0.001, y + 0.001),
                        (x - 0.001, y + 0.001),
                        (x - 0.001, y - 0.001),
                    ]),
                    vec![],
                )
            })
            .collect();
        let build_tiler = || {
            TilerBuilder::new(h3o::Resolution::Three)
                .containment_mode(ContainmentMode::IntersectsBoundary)
                .disable_transmeridian_heuristic()
                .build()
        };
        let raw_count: usize = polygons
            .iter()
            .map(|polygon| {
                let mut tiler = build_tiler();
                tiler.add(polygon.clone()).unwrap();
                tiler.into_coverage().count()
            })
            .sum();
        let mut combined = build_tiler();
        combined.add_batch(polygons.clone()).unwrap();
        let mut expected: Vec<_> = combined
            .into_coverage()
            .map(|cell| cell.to_string())
            .collect();
        expected.sort_unstable();
        expected.dedup();
        assert!(raw_count > expected.len());
        let geometry = from_geo(&geo::Geometry::MultiPolygon(geo::MultiPolygon::new(
            polygons,
        )))
        .unwrap();
        let inputs = json!({"geometry":geometry, "resolution":3, "containment":"intersects-boundary", "max_cells":expected.len()});
        let actual = cpu::run(move || execute(Operation::PolygonToH3Cells, &inputs))
            .await
            .unwrap();
        assert_eq!(actual[0].1, json!(expected));
    }
}
