//! Planar overlay, topology, validity, and buffer nodes for native Geometry values.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    geometry::{GeometryKind, marker},
    json::json,
};

#[cfg(feature = "execute")]
use super::{
    charge_topology_validation_work, check_topology_validation, cpu, ensure_geometry_pair_budget,
    topology_is_valid, topology_validation_error, topology_validation_errors,
};
#[cfg(feature = "execute")]
use flow_like_geometry::{from_geo, to_geo};
#[cfg(feature = "execute")]
use flow_like_types::{
    Result, Value, anyhow, bail,
    geometry::{MAX_GEOMETRY_POSITIONS, canonicalize_geometry},
};
#[cfg(feature = "execute")]
use geo::{
    BooleanOps, Buffer, Coord, CoordsIter, Distance, Euclidean, Geometry, GeometryCollection,
    LineString, MultiLineString, MultiPoint, MultiPolygon, Polygon, Relate,
    algorithm::{
        buffer::{BufferStyle, LineCap, LineJoin},
        unary_union,
    },
};
#[cfg(feature = "execute")]
use std::sync::atomic::{AtomicUsize, Ordering};

const DEFAULT_STYLE_ANGLE_DEGREES: f64 = 11.459_155_902_616_466;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Union,
    Difference,
    SymmetricDifference,
    UnaryUnion,
    ClipLine,
    Covers,
    CoveredBy,
    Touches,
    Crosses,
    Overlaps,
    TopologicallyEquals,
    Disjoint,
    DWithin,
    RelatePattern,
    IsTopologicallyValid,
    ValidityReason,
    ValidationErrors,
    IsEmpty,
    Repair,
    PlanarBuffer,
}

fn geometry_input<'a>(node: &'a mut Node, name: &str, kind: Option<GeometryKind>) -> &'a mut Pin {
    let pin = node.add_input_pin(
        name,
        name,
        "Validated two-dimensional WGS 84 GeoJSON geometry",
        VariableType::Geometry,
    );
    pin.schema = kind.map(|kind| marker(kind).to_string());
    pin
}

fn geometry_output(node: &mut Node, kind: Option<GeometryKind>) {
    let pin = node.add_output_pin(
        "geometry_out",
        "Geometry",
        "Validated two-dimensional WGS 84 GeoJSON geometry",
        VariableType::Geometry,
    );
    pin.schema = kind.map(|kind| marker(kind).to_string());
}

fn data_output<'a>(
    node: &'a mut Node,
    name: &str,
    ty: VariableType,
    description: &str,
) -> &'a mut Pin {
    node.add_output_pin(name, name, description, ty)
}

fn definition(operation: Operation) -> Node {
    use Operation::*;
    let (id, alias, title, description) = match operation {
        Union => (
            "geometry_union",
            "booleanUnion",
            "Geometry Union (Planar)",
            "Combines Polygon or MultiPolygon regions in the longitude/latitude coordinate plane. Returns a MultiPolygon.",
        ),
        Difference => (
            "geometry_difference",
            "booleanDifference",
            "Geometry Difference (Planar)",
            "Subtracts Polygon or MultiPolygon B from A in the longitude/latitude coordinate plane. Returns a MultiPolygon.",
        ),
        SymmetricDifference => (
            "geometry_symmetric_difference",
            "symmetricDifference",
            "Geometry Symmetric Difference (Planar)",
            "Returns Polygon or MultiPolygon regions belonging to exactly one input. Returns a MultiPolygon.",
        ),
        UnaryUnion => (
            "geometry_unary_union",
            "unaryUnion",
            "Dissolve Geometries (Planar)",
            "Efficiently dissolves an array of Polygon and MultiPolygon values. An empty array returns an empty MultiPolygon.",
        ),
        ClipLine => (
            "geometry_clip_line",
            "clipLine",
            "Clip Line by Polygon (Planar)",
            "Returns the portions of a LineString or MultiLineString inside a Polygon or MultiPolygon mask, including its boundary.",
        ),
        Covers => (
            "geometry_covers",
            "covers",
            "Geometry Covers (Planar)",
            "Tests whether every point of B lies in the interior or boundary of A using DE-9IM semantics.",
        ),
        CoveredBy => (
            "geometry_covered_by",
            "coveredBy",
            "Geometry Covered By (Planar)",
            "Tests whether every point of A lies in the interior or boundary of B using DE-9IM semantics.",
        ),
        Touches => (
            "geometry_touches",
            "touches",
            "Geometry Touches (Planar)",
            "Tests whether the inputs share boundary points but have disjoint interiors using DE-9IM semantics.",
        ),
        Crosses => (
            "geometry_crosses",
            "crosses",
            "Geometry Crosses (Planar)",
            "Tests whether the inputs cross using DE-9IM semantics.",
        ),
        Overlaps => (
            "geometry_overlaps",
            "overlaps",
            "Geometry Overlaps (Planar)",
            "Tests whether same-dimensional inputs partly overlap without either covering the other.",
        ),
        TopologicallyEquals => (
            "geometry_topologically_equals",
            "topologicallyEquals",
            "Geometry Topologically Equals (Planar)",
            "Tests whether both inputs describe the same point set using DE-9IM semantics. Coordinate order may differ.",
        ),
        Disjoint => (
            "geometry_disjoint",
            "disjoint",
            "Geometry Disjoint (Planar)",
            "Tests whether the inputs share no point using DE-9IM semantics.",
        ),
        DWithin => (
            "geometry_d_within",
            "dWithin",
            "Geometry Within Distance (Degrees)",
            "Tests whether the shortest planar distance between two non-empty geometries is at most the supplied coordinate-degree distance.",
        ),
        RelatePattern => (
            "geometry_relate_pattern",
            "relatePattern",
            "Geometry Relate Pattern (Planar)",
            "Tests a DE-9IM relation pattern. The pattern has nine characters chosen from T, F, *, 0, 1, and 2.",
        ),
        IsTopologicallyValid => (
            "geometry_is_topologically_valid",
            "isTopologicallyValid",
            "Is Geometry Topologically Valid",
            "Checks OGC Simple Feature topology after the Geometry value has passed structural validation.",
        ),
        ValidityReason => (
            "geometry_validity_reason",
            "validityReason",
            "Geometry Validity Reason",
            "Returns the first OGC topology error, or an empty string when the geometry is topologically valid.",
        ),
        ValidationErrors => (
            "geometry_validation_errors",
            "validationErrors",
            "Geometry Validation Errors",
            "Returns up to 256 topology errors. If more exist, the final array entry reports truncation. A valid geometry returns an empty array.",
        ),
        IsEmpty => (
            "geometry_is_empty",
            "isEmpty",
            "Is Geometry Empty",
            "Tests whether a multi-geometry or GeometryCollection contains no coordinate positions.",
        ),
        Repair => (
            "geometry_repair",
            "repair",
            "Repair Geometry",
            "Repairs a limited set of topology defects: consecutive duplicate coordinates, duplicate MultiPoint members, overlapping MultiPolygon parts, and polygon self-intersections that planar overlay can resolve. GeometryCollection members are repaired independently. Other defects return an error.",
        ),
        PlanarBuffer => (
            "geometry_planar_buffer",
            "planarBuffer",
            "Geometry Buffer (Degrees)",
            "Buffers a geometry in coordinate degrees. The operation is planar, does not wrap at the antimeridian, and rejects results outside WGS 84 coordinate bounds.",
        ),
    };

    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");

    match operation {
        Union | Difference | SymmetricDifference => {
            geometry_input(&mut node, "a", None);
            geometry_input(&mut node, "b", None);
            geometry_output(&mut node, Some(GeometryKind::MultiPolygon));
        }
        UnaryUnion => {
            geometry_input(&mut node, "geometries", None)
                .set_value_type(ValueType::Array)
                .set_default_value(Some(json!([])));
            geometry_output(&mut node, Some(GeometryKind::MultiPolygon));
        }
        ClipLine => {
            geometry_input(&mut node, "line", None);
            geometry_input(&mut node, "mask", None);
            geometry_output(&mut node, Some(GeometryKind::MultiLineString));
        }
        Covers | CoveredBy | Touches | Crosses | Overlaps | TopologicallyEquals | Disjoint => {
            geometry_input(&mut node, "a", None);
            geometry_input(&mut node, "b", None);
            data_output(
                &mut node,
                "result",
                VariableType::Boolean,
                "Planar predicate result",
            );
        }
        DWithin => {
            geometry_input(&mut node, "a", None);
            geometry_input(&mut node, "b", None);
            node.add_input_pin(
                "distance",
                "distance",
                "Maximum planar distance in coordinate degrees",
                VariableType::Float,
            )
            .set_default_value(Some(json!(0.0)))
            .set_options(PinOptions::new().set_range((0.0, 403.0)).build());
            data_output(
                &mut node,
                "result",
                VariableType::Boolean,
                "Whether the geometries are within the requested distance",
            );
        }
        RelatePattern => {
            geometry_input(&mut node, "a", None);
            geometry_input(&mut node, "b", None);
            node.add_input_pin(
                "pattern",
                "pattern",
                "Nine-character DE-9IM pattern using T, F, *, 0, 1, and 2",
                VariableType::String,
            )
            .set_default_value(Some(json!("*********")));
            data_output(
                &mut node,
                "result",
                VariableType::Boolean,
                "Whether the relation matches the pattern",
            );
        }
        IsTopologicallyValid => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "result",
                VariableType::Boolean,
                "Whether the geometry satisfies OGC topology rules",
            );
        }
        ValidityReason => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "reason",
                VariableType::String,
                "First topology error, or an empty string",
            );
        }
        ValidationErrors => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "errors",
                VariableType::String,
                "Topology validation errors, capped at 256 entries plus a truncation notice",
            )
            .set_value_type(ValueType::Array);
        }
        IsEmpty => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "result",
                VariableType::Boolean,
                "Whether the geometry has no coordinate positions",
            );
        }
        Repair => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, None);
        }
        PlanarBuffer => {
            geometry_input(&mut node, "geometry", None);
            node.add_input_pin(
                "distance",
                "distance",
                "Signed buffer distance in coordinate degrees. Negative distances shrink polygonal inputs.",
                VariableType::Float,
            )
            .set_default_value(Some(json!(0.0)))
            .set_options(PinOptions::new().set_range((-360.0, 360.0)).build());
            node.add_input_pin(
                "cap",
                "cap",
                "Line endpoint style: Round, Square, or Butt",
                VariableType::String,
            )
            .set_default_value(Some(json!("Round")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["Round".into(), "Square".into(), "Butt".into()])
                    .build(),
            );
            node.add_input_pin(
                "join",
                "join",
                "Corner style: Round, Miter, or Bevel",
                VariableType::String,
            )
            .set_default_value(Some(json!("Round")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["Round".into(), "Miter".into(), "Bevel".into()])
                    .build(),
            );
            node.add_input_pin(
                "arc_step_degrees",
                "arc_step_degrees",
                "Maximum angular step used to approximate round caps and joins",
                VariableType::Float,
            )
            .set_default_value(Some(json!(DEFAULT_STYLE_ANGLE_DEGREES)))
            .set_options(PinOptions::new().set_range((1.8, 45.0)).build());
            node.add_input_pin(
                "miter_min_angle_degrees",
                "miter_min_angle_degrees",
                "Minimum corner angle that retains a miter instead of beveling it",
                VariableType::Float,
            )
            .set_default_value(Some(json!(DEFAULT_STYLE_ANGLE_DEGREES)))
            .set_options(PinOptions::new().set_range((1.8, 178.2)).build());
            geometry_output(&mut node, Some(GeometryKind::MultiPolygon));
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
macro_rules! ensure {
    ($condition:expr, $($message:tt)*) => {
        if !$condition { bail!($($message)*); }
    };
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
    ensure!(value.is_finite(), "{name} must be finite");
    Ok(value)
}

#[cfg(feature = "execute")]
fn text<'a>(inputs: &'a Value, name: &str) -> Result<&'a str> {
    input(inputs, name)?
        .as_str()
        .ok_or_else(|| anyhow!("{name} must be a string"))
}

#[cfg(feature = "execute")]
fn raw_geometry(value: &Value) -> Result<Geometry<f64>> {
    to_geo(&canonicalize_geometry(value, None)?)
}

#[cfg(feature = "execute")]
fn checked_geometry(value: &Value) -> Result<Geometry<f64>> {
    let geometry = raw_geometry(value)?;
    check_topology_validation(&geometry, "Invalid geometry for spatial operation")?;
    Ok(geometry)
}

#[cfg(feature = "execute")]
fn polygon_set(geometry: Geometry<f64>) -> Result<MultiPolygon<f64>> {
    match geometry {
        Geometry::Polygon(polygon) => Ok(MultiPolygon(vec![polygon])),
        Geometry::MultiPolygon(polygons) => Ok(polygons),
        _ => bail!("This operation requires Polygon or MultiPolygon"),
    }
}

#[cfg(feature = "execute")]
fn line_set(geometry: Geometry<f64>) -> Result<MultiLineString<f64>> {
    match geometry {
        Geometry::LineString(line) => Ok(MultiLineString(vec![line])),
        Geometry::MultiLineString(lines) => Ok(lines),
        _ => bail!("This operation requires LineString or MultiLineString"),
    }
}

#[cfg(feature = "execute")]
fn encoded_geometry(geometry: Geometry<f64>, kind: GeometryKind) -> Result<Value> {
    check_topology_validation(&geometry, "Spatial operation produced invalid geometry")?;
    Ok(canonicalize_geometry(&from_geo(&geometry)?, Some(kind))?)
}

#[cfg(feature = "execute")]
fn geometry_result(
    geometry: Geometry<f64>,
    kind: GeometryKind,
) -> Result<Vec<(&'static str, Value)>> {
    Ok(vec![("geometry_out", encoded_geometry(geometry, kind)?)])
}

#[cfg(feature = "execute")]
fn any_geometry_result(geometry: Geometry<f64>) -> Result<Vec<(&'static str, Value)>> {
    check_topology_validation(&geometry, "Geometry repair produced invalid geometry")?;
    Ok(vec![(
        "geometry_out",
        canonicalize_geometry(&from_geo(&geometry)?, None)?,
    )])
}

#[cfg(feature = "execute")]
fn same_coord(a: Coord<f64>, b: Coord<f64>) -> bool {
    a.x == b.x && a.y == b.y
}

#[cfg(feature = "execute")]
fn repaired_ring_segment_count(ring: &LineString<f64>) -> usize {
    let mut first = None;
    let mut previous = None;
    let mut positions = 0usize;
    for coordinate in ring.0.iter().copied() {
        if previous.is_none_or(|value| !same_coord(value, coordinate)) {
            first.get_or_insert(coordinate);
            previous = Some(coordinate);
            positions = positions.saturating_add(1);
        }
    }
    if first.is_some_and(|first| previous.is_some_and(|last| !same_coord(last, first))) {
        positions = positions.saturating_add(1);
    }
    positions.saturating_sub(1)
}

#[cfg(feature = "execute")]
fn charge_repaired_polygon_work(work: &mut usize, polygon: &Polygon<f64>) -> Result<usize> {
    let exterior = repaired_ring_segment_count(polygon.exterior());
    charge_topology_validation_work(work, exterior, exterior)?;
    let mut preceding_holes = 0usize;
    let mut polygon_segments = exterior;
    for hole in polygon.interiors() {
        let segments = repaired_ring_segment_count(hole);
        charge_topology_validation_work(work, segments, segments)?;
        charge_topology_validation_work(work, exterior, segments)?;
        charge_topology_validation_work(work, preceding_holes, segments)?;
        preceding_holes = preceding_holes.saturating_add(segments);
        polygon_segments = polygon_segments.saturating_add(segments);
    }
    Ok(polygon_segments)
}

#[cfg(feature = "execute")]
fn charge_repair_validation_work(work: &mut usize, geometry: &Geometry<f64>) -> Result<()> {
    match geometry {
        Geometry::Polygon(polygon) => charge_repaired_polygon_work(work, polygon).map(|_| ()),
        Geometry::MultiPolygon(polygons) => {
            let mut preceding_polygons = 0usize;
            for polygon in &polygons.0 {
                let segments = charge_repaired_polygon_work(work, polygon)?;
                charge_topology_validation_work(work, preceding_polygons, segments)?;
                preceding_polygons = preceding_polygons.saturating_add(segments);
            }
            Ok(())
        }
        Geometry::GeometryCollection(collection) => {
            for member in &collection.0 {
                charge_repair_validation_work(work, member)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(feature = "execute")]
fn repair_validation_work(geometry: &Geometry<f64>) -> Result<usize> {
    let mut work = 0usize;
    charge_repair_validation_work(&mut work, geometry)?;
    Ok(work)
}

/// Remove only adjacent duplicate positions. Removing non-adjacent positions can change topology.
#[cfg(feature = "execute")]
fn clean_line(line: LineString<f64>, ring: bool) -> Result<LineString<f64>> {
    let mut coordinates: Vec<Coord<f64>> = Vec::with_capacity(line.0.len() + usize::from(ring));
    for coordinate in line.0 {
        if coordinates
            .last()
            .is_none_or(|previous| !same_coord(*previous, coordinate))
        {
            coordinates.push(coordinate);
        }
    }

    if ring {
        let first = coordinates
            .first()
            .copied()
            .ok_or_else(|| anyhow!("Cannot repair an empty polygon ring"))?;
        if coordinates
            .last()
            .is_none_or(|last| !same_coord(*last, first))
        {
            coordinates.push(first);
        }
        // Only the first three distinct vertices matter. Keeping every unique
        // coordinate would make a maximum-size ring quadratic to inspect.
        let mut distinct = Vec::with_capacity(3);
        for coordinate in coordinates.iter().copied().take(coordinates.len() - 1) {
            if !distinct
                .iter()
                .any(|existing| same_coord(*existing, coordinate))
            {
                distinct.push(coordinate);
                if distinct.len() == 3 {
                    break;
                }
            }
        }
        ensure!(
            distinct.len() >= 3,
            "Cannot safely repair a polygon ring with fewer than three distinct vertices"
        );
    } else {
        ensure!(
            coordinates.len() >= 2,
            "Cannot safely repair a LineString with fewer than two distinct positions"
        );
    }
    Ok(LineString::new(coordinates))
}

#[cfg(feature = "execute")]
fn clean_polygon(polygon: Polygon<f64>) -> Result<Polygon<f64>> {
    let exterior = clean_line(polygon.exterior().clone(), true)?;
    let work = polygon
        .interiors()
        .iter()
        .fold(0usize, |count, ring| count.saturating_add(ring.0.len()));
    let interiors = cpu::map_ordered(polygon.interiors(), work, |_, ring| {
        clean_line(ring.clone(), true)
    })
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    Ok(Polygon::new(exterior, interiors))
}

#[cfg(feature = "execute")]
fn overlay_repair(polygons: MultiPolygon<f64>) -> Result<MultiPolygon<f64>> {
    let Some(original_error) = topology_validation_error(&polygons)? else {
        return Ok(polygons);
    };

    let overlaid = unary_union(polygons.0.iter());
    if !overlaid.0.is_empty() && topology_is_valid(&overlaid)? {
        return Ok(overlaid);
    }
    let buffered = polygons.buffer(0.0);
    if !buffered.0.is_empty() && topology_is_valid(&buffered)? {
        return Ok(buffered);
    }
    bail!("Geometry cannot be safely repaired: {original_error}")
}

#[cfg(feature = "execute")]
fn repair_polygon(polygon: Polygon<f64>) -> Result<Geometry<f64>> {
    let mut repaired = overlay_repair(MultiPolygon(vec![clean_polygon(polygon)?]))?;
    if repaired.0.len() == 1 {
        Ok(Geometry::Polygon(repaired.0.remove(0)))
    } else {
        Ok(Geometry::MultiPolygon(repaired))
    }
}

#[cfg(feature = "execute")]
fn repair_geometry_inner(
    geometry: Geometry<f64>,
    emitted_positions: &AtomicUsize,
) -> Result<Geometry<f64>> {
    // Charge the complete collection before any member can consume the full
    // validation and overlay budget on a worker of its own.
    let validation_work = repair_validation_work(&geometry)?;
    let repaired = match geometry {
        Geometry::Point(point) => Geometry::Point(point),
        Geometry::MultiPoint(points) => {
            use std::collections::HashSet;

            let mut unique = Vec::with_capacity(points.0.len());
            let mut seen = HashSet::with_capacity(points.0.len());
            for point in points.0 {
                let key = (
                    if point.x() == 0.0 {
                        0
                    } else {
                        point.x().to_bits()
                    },
                    if point.y() == 0.0 {
                        0
                    } else {
                        point.y().to_bits()
                    },
                );
                if seen.insert(key) {
                    unique.push(point);
                }
            }
            Geometry::MultiPoint(MultiPoint(unique))
        }
        Geometry::LineString(line) => Geometry::LineString(clean_line(line, false)?),
        Geometry::MultiLineString(lines) => {
            let work = lines.coords_count();
            let cleaned =
                cpu::map_ordered(&lines.0, work, |_, line| clean_line(line.clone(), false))
                    .into_iter()
                    .collect::<Result<Vec<_>>>()?;
            Geometry::MultiLineString(MultiLineString(cleaned))
        }
        Geometry::Polygon(polygon) => repair_polygon(polygon)?,
        Geometry::MultiPolygon(polygons) => {
            let work = polygons.coords_count();
            let cleaned = cpu::map_ordered(&polygons.0, work, |_, polygon| {
                clean_polygon(polygon.clone())
            })
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
            Geometry::MultiPolygon(overlay_repair(MultiPolygon(cleaned))?)
        }
        Geometry::GeometryCollection(collection) => {
            let work = collection.coords_count().max(validation_work);
            let repaired = cpu::map_ordered(&collection.0, work, |_, member| {
                repair_geometry_inner(member.clone(), emitted_positions)
            })
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
            Geometry::GeometryCollection(GeometryCollection(repaired))
        }
        _ => bail!("Unsupported geometry variant for repair"),
    };
    if !matches!(&repaired, Geometry::GeometryCollection(_)) {
        let positions = repaired.coords_count();
        emitted_positions
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current
                    .checked_add(positions)
                    .filter(|total| *total <= MAX_GEOMETRY_POSITIONS)
            })
            .map_err(|_| {
                anyhow!("Geometry repair exceeds the position limit of {MAX_GEOMETRY_POSITIONS}")
            })?;
    }
    ensure!(
        repaired.coords_count() <= MAX_GEOMETRY_POSITIONS,
        "Geometry repair exceeds the position limit of {MAX_GEOMETRY_POSITIONS}"
    );
    check_topology_validation(&repaired, "Geometry cannot be safely repaired")?;
    Ok(repaired)
}

#[cfg(feature = "execute")]
fn repair_geometry(geometry: Geometry<f64>) -> Result<Geometry<f64>> {
    repair_geometry_inner(geometry, &AtomicUsize::new(0))
}

#[cfg(feature = "execute")]
fn validation_errors(inputs: &Value) -> Result<Vec<String>> {
    topology_validation_errors(&raw_geometry(input(inputs, "geometry")?)?)
}

#[cfg(feature = "execute")]
#[derive(Default)]
struct BufferInputCounts {
    points: usize,
    line_positions: usize,
    lines: usize,
    area_positions: usize,
}

#[cfg(feature = "execute")]
fn count_buffer_input(geometry: &Geometry<f64>, counts: &mut BufferInputCounts) {
    match geometry {
        Geometry::Point(_) => counts.points = counts.points.saturating_add(1),
        Geometry::MultiPoint(points) => {
            counts.points = counts.points.saturating_add(points.0.len());
        }
        Geometry::Line(_) => {
            counts.line_positions = counts.line_positions.saturating_add(2);
            counts.lines = counts.lines.saturating_add(1);
        }
        Geometry::LineString(line) => {
            counts.line_positions = counts.line_positions.saturating_add(line.0.len());
            counts.lines = counts.lines.saturating_add(1);
        }
        Geometry::MultiLineString(lines) => {
            for line in &lines.0 {
                counts.line_positions = counts.line_positions.saturating_add(line.0.len());
                counts.lines = counts.lines.saturating_add(1);
            }
        }
        Geometry::Polygon(polygon) => {
            counts.area_positions = counts.area_positions.saturating_add(polygon.coords_count());
        }
        Geometry::MultiPolygon(polygons) => {
            counts.area_positions = counts
                .area_positions
                .saturating_add(polygons.coords_count());
        }
        Geometry::GeometryCollection(collection) => {
            for member in &collection.0 {
                count_buffer_input(member, counts);
            }
        }
        Geometry::Rect(_) => {
            counts.area_positions = counts.area_positions.saturating_add(5);
        }
        Geometry::Triangle(_) => {
            counts.area_positions = counts.area_positions.saturating_add(4);
        }
    }
}

#[cfg(feature = "execute")]
fn add_buffer_position_estimate(
    estimate: &mut usize,
    count: usize,
    expansion: usize,
) -> Result<()> {
    let remaining = MAX_GEOMETRY_POSITIONS.saturating_sub(*estimate);
    if count != 0 && expansion > remaining / count {
        bail!(
            "Planar buffer may construct more than {MAX_GEOMETRY_POSITIONS} positions for the selected cap, join, and arc step; increase arc_step_degrees, choose non-round styles, or reduce the input"
        );
    }
    *estimate += count * expansion;
    Ok(())
}

/// i_overlay can add an arc of up to PI radians at every join and two such
/// arcs around a point. Bound that temporary construction before it allocates.
#[cfg(feature = "execute")]
fn ensure_buffer_position_budget(
    geometry: &Geometry<f64>,
    distance: f64,
    arc_step: f64,
    cap: &str,
    join: &str,
) -> Result<()> {
    let mut counts = BufferInputCounts::default();
    count_buffer_input(geometry, &mut counts);

    let half_circle_steps = (std::f64::consts::PI / arc_step).ceil() as usize;
    let join_expansion = match join {
        "Round" => half_circle_steps.saturating_add(3),
        "Miter" => 6,
        "Bevel" => 4,
        _ => unreachable!("join style was validated before buffer preflight"),
    };
    let cap_expansion = match cap {
        "Round" => half_circle_steps.saturating_add(1),
        "Square" => 3,
        "Butt" => 1,
        _ => unreachable!("cap style was validated before buffer preflight"),
    };
    let point_expansion = match cap {
        "Round" => half_circle_steps.saturating_mul(2).saturating_add(1),
        "Square" => 5,
        "Butt" => 0,
        _ => unreachable!("cap style was validated before buffer preflight"),
    };

    let mut estimate = 0usize;
    add_buffer_position_estimate(&mut estimate, counts.area_positions, join_expansion)?;
    if distance > 0.0 {
        add_buffer_position_estimate(&mut estimate, counts.points, point_expansion)?;
        add_buffer_position_estimate(&mut estimate, counts.line_positions, join_expansion)?;
        add_buffer_position_estimate(&mut estimate, counts.lines.saturating_mul(2), cap_expansion)?;
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn execute(operation: Operation, inputs: &Value) -> Result<Vec<(&'static str, Value)>> {
    use Operation::*;
    match operation {
        Union | Difference | SymmetricDifference => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            ensure_geometry_pair_budget(&a, &b, "Polygon overlay")?;
            let a = polygon_set(a)?;
            let b = polygon_set(b)?;
            let result = match operation {
                Union => a.union(&b),
                Difference => a.difference(&b),
                SymmetricDifference => a.xor(&b),
                _ => unreachable!(),
            };
            geometry_result(result.into(), GeometryKind::MultiPolygon)
        }
        UnaryUnion => {
            let values = input(inputs, "geometries")?
                .as_array()
                .ok_or_else(|| anyhow!("geometries must be an array"))?;
            let mut polygons = Vec::new();
            let mut coordinate_count = 0usize;
            for value in values {
                let geometry = checked_geometry(value)?;
                coordinate_count = coordinate_count.saturating_add(geometry.coords_count());
                ensure!(
                    coordinate_count <= MAX_GEOMETRY_POSITIONS,
                    "Unary union inputs exceed the Geometry position limit of {MAX_GEOMETRY_POSITIONS}"
                );
                polygons.extend(polygon_set(geometry)?.0);
            }
            let result = if polygons.is_empty() {
                MultiPolygon(vec![])
            } else {
                unary_union(polygons.iter())
            };
            geometry_result(result.into(), GeometryKind::MultiPolygon)
        }
        ClipLine => {
            let line = checked_geometry(input(inputs, "line")?)?;
            let mask = checked_geometry(input(inputs, "mask")?)?;
            ensure_geometry_pair_budget(&line, &mask, "Line clipping")?;
            let line = line_set(line)?;
            let mask = polygon_set(mask)?;
            geometry_result(
                mask.clip(&line, false).into(),
                GeometryKind::MultiLineString,
            )
        }
        Covers | CoveredBy | Touches | Crosses | Overlaps | TopologicallyEquals | Disjoint => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            let operation_name = match operation {
                Covers => "Geometry covers predicate",
                CoveredBy => "Geometry covered-by predicate",
                Touches => "Geometry touches predicate",
                Crosses => "Geometry crosses predicate",
                Overlaps => "Geometry overlaps predicate",
                TopologicallyEquals => "Geometry topological equality predicate",
                Disjoint => "Geometry disjoint predicate",
                _ => unreachable!(),
            };
            ensure_geometry_pair_budget(&a, &b, operation_name)?;
            let relation = a.relate(&b);
            let result = match operation {
                Covers => relation.is_covers(),
                CoveredBy => relation.is_coveredby(),
                Touches => relation.is_touches(),
                Crosses => relation.is_crosses(),
                Overlaps => relation.is_overlaps(),
                TopologicallyEquals => relation.is_equal_topo(),
                Disjoint => relation.is_disjoint(),
                _ => unreachable!(),
            };
            Ok(vec![("result", json!(result))])
        }
        DWithin => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            ensure_geometry_pair_budget(&a, &b, "Geometry within-distance predicate")?;
            let distance = number(inputs, "distance")?;
            ensure!(
                (0.0..=403.0).contains(&distance),
                "distance must be between 0 and 403 coordinate degrees"
            );
            ensure!(
                a.coords_iter().next().is_some() && b.coords_iter().next().is_some(),
                "Empty geometries have no distance"
            );
            Ok(vec![(
                "result",
                json!(Euclidean.distance(&a, &b) <= distance),
            )])
        }
        RelatePattern => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            ensure_geometry_pair_budget(&a, &b, "DE-9IM geometry relation")?;
            let pattern = text(inputs, "pattern")?;
            let result = a
                .relate(&b)
                .matches(pattern)
                .map_err(|error| anyhow!("Invalid DE-9IM pattern: {error}"))?;
            Ok(vec![("result", json!(result))])
        }
        IsTopologicallyValid => {
            let geometry = raw_geometry(input(inputs, "geometry")?)?;
            Ok(vec![("result", json!(topology_is_valid(&geometry)?))])
        }
        ValidityReason => {
            let geometry = raw_geometry(input(inputs, "geometry")?)?;
            let reason = topology_validation_error(&geometry)?.unwrap_or_default();
            Ok(vec![("reason", json!(reason))])
        }
        ValidationErrors => Ok(vec![("errors", json!(validation_errors(inputs)?))]),
        IsEmpty => Ok(vec![(
            "result",
            json!(
                raw_geometry(input(inputs, "geometry")?)?
                    .coords_iter()
                    .next()
                    .is_none()
            ),
        )]),
        Repair => any_geometry_result(repair_geometry(raw_geometry(input(inputs, "geometry")?)?)?),
        PlanarBuffer => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            let distance = number(inputs, "distance")?;
            ensure!(
                (-360.0..=360.0).contains(&distance),
                "distance must be between -360 and 360 degrees"
            );
            let arc_step = number(inputs, "arc_step_degrees")?;
            ensure!(
                (1.8..=45.0).contains(&arc_step),
                "arc_step_degrees must be between 1.8 and 45"
            );
            let miter_angle = number(inputs, "miter_min_angle_degrees")?;
            ensure!(
                (1.8..=178.2).contains(&miter_angle),
                "miter_min_angle_degrees must be between 1.8 and 178.2"
            );
            let arc_step = arc_step.to_radians();
            let cap = text(inputs, "cap")?;
            let join = text(inputs, "join")?;
            ensure!(
                matches!(cap, "Round" | "Square" | "Butt"),
                "Unknown buffer cap style: {cap}"
            );
            ensure!(
                matches!(join, "Round" | "Miter" | "Bevel"),
                "Unknown buffer join style: {join}"
            );
            ensure_buffer_position_budget(&geometry, distance, arc_step, cap, join)?;
            let mut style = BufferStyle::new(distance);
            style = style.line_cap(match cap {
                "Round" => LineCap::Round(arc_step),
                "Square" => LineCap::Square,
                "Butt" => LineCap::Butt,
                _ => unreachable!("cap style was validated before buffer construction"),
            });
            style = style.line_join(match join {
                "Round" => LineJoin::Round(arc_step),
                "Miter" => LineJoin::Miter(miter_angle.to_radians()),
                "Bevel" => LineJoin::Bevel,
                _ => unreachable!("join style was validated before buffer construction"),
            });
            geometry_result(
                geometry.buffer_with_style(style).into(),
                GeometryKind::MultiPolygon,
            )
        }
    }
}

#[cfg(feature = "execute")]
async fn run_operation(operation: Operation, context: &mut ExecutionContext) -> Result<()> {
    let mut values = flow_like_types::json::Map::new();
    for pin in definition(operation)
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input)
    {
        values.insert(
            pin.name.clone(),
            context.evaluate_pin::<Value>(&pin.name).await?,
        );
    }
    let outputs = super::cpu::run(move || execute(operation, &Value::Object(values))).await?;
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
pub struct GeometryUnionNode;
implement_node!(GeometryUnionNode, Operation::Union);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDifferenceNode;
implement_node!(GeometryDifferenceNode, Operation::Difference);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySymmetricDifferenceNode;
implement_node!(
    GeometrySymmetricDifferenceNode,
    Operation::SymmetricDifference
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryUnaryUnionNode;
implement_node!(GeometryUnaryUnionNode, Operation::UnaryUnion);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryClipLineNode;
implement_node!(GeometryClipLineNode, Operation::ClipLine);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCoversNode;
implement_node!(GeometryCoversNode, Operation::Covers);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCoveredByNode;
implement_node!(GeometryCoveredByNode, Operation::CoveredBy);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryTouchesNode;
implement_node!(GeometryTouchesNode, Operation::Touches);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCrossesNode;
implement_node!(GeometryCrossesNode, Operation::Crosses);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryOverlapsNode;
implement_node!(GeometryOverlapsNode, Operation::Overlaps);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryTopologicallyEqualsNode;
implement_node!(
    GeometryTopologicallyEqualsNode,
    Operation::TopologicallyEquals
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDisjointNode;
implement_node!(GeometryDisjointNode, Operation::Disjoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDWithinNode;
implement_node!(GeometryDWithinNode, Operation::DWithin);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryRelatePatternNode;
implement_node!(GeometryRelatePatternNode, Operation::RelatePattern);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryIsTopologicallyValidNode;
implement_node!(
    GeometryIsTopologicallyValidNode,
    Operation::IsTopologicallyValid
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryValidityReasonNode;
implement_node!(GeometryValidityReasonNode, Operation::ValidityReason);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryValidationErrorsNode;
implement_node!(GeometryValidationErrorsNode, Operation::ValidationErrors);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryIsEmptyNode;
implement_node!(GeometryIsEmptyNode, Operation::IsEmpty);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryRepairNode;
implement_node!(GeometryRepairNode, Operation::Repair);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPlanarBufferNode;
implement_node!(GeometryPlanarBufferNode, Operation::PlanarBuffer);

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use geo::{Area, Validation};

    fn output(operation: Operation, inputs: Value, name: &str) -> Value {
        execute(operation, &inputs)
            .unwrap()
            .into_iter()
            .find(|(output_name, _)| *output_name == name)
            .unwrap()
            .1
    }

    fn square(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Value {
        json!({
            "type": "Polygon",
            "coordinates": [[
                [min_x, min_y],
                [max_x, min_y],
                [max_x, max_y],
                [min_x, max_y],
                [min_x, min_y]
            ]]
        })
    }

    fn point(x: f64, y: f64) -> Value {
        json!({"type": "Point", "coordinates": [x, y]})
    }

    fn area(value: &Value) -> f64 {
        to_geo(value).unwrap().unsigned_area()
    }

    #[test]
    fn polygon_boolean_operations_have_set_semantics() {
        let inputs = json!({"a": square(0.0, 0.0, 2.0, 2.0), "b": square(1.0, 1.0, 3.0, 3.0)});
        assert_eq!(
            area(&output(Operation::Union, inputs.clone(), "geometry_out")),
            7.0
        );
        assert_eq!(
            area(&output(
                Operation::Difference,
                inputs.clone(),
                "geometry_out"
            )),
            3.0
        );
        assert_eq!(
            area(&output(
                Operation::SymmetricDifference,
                inputs,
                "geometry_out"
            )),
            6.0
        );
    }

    #[test]
    fn unary_union_accepts_polygons_multipolygons_and_empty_arrays() {
        let dissolved = output(
            Operation::UnaryUnion,
            json!({"geometries": [square(0.0, 0.0, 1.0, 1.0), square(1.0, 0.0, 2.0, 1.0)]}),
            "geometry_out",
        );
        assert_eq!(area(&dissolved), 2.0);
        assert_eq!(dissolved["coordinates"].as_array().unwrap().len(), 1);

        let empty = output(
            Operation::UnaryUnion,
            json!({"geometries": []}),
            "geometry_out",
        );
        assert_eq!(empty, json!({"type": "MultiPolygon", "coordinates": []}));
    }

    #[test]
    fn clip_line_returns_only_segments_inside_mask() {
        let clipped = output(
            Operation::ClipLine,
            json!({
                "line": {"type": "LineString", "coordinates": [[-1.0, 0.5], [2.0, 0.5]]},
                "mask": square(0.0, 0.0, 1.0, 1.0)
            }),
            "geometry_out",
        );
        assert_eq!(clipped["type"], "MultiLineString");
        assert_eq!(clipped["coordinates"], json!([[[0.0, 0.5], [1.0, 0.5]]]));
    }

    #[test]
    fn de9im_predicates_cover_boundaries_and_set_relations() {
        let region = square(0.0, 0.0, 2.0, 2.0);
        let boundary_point = point(0.0, 1.0);
        assert_eq!(
            output(
                Operation::Covers,
                json!({"a": region, "b": boundary_point}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::CoveredBy,
                json!({"a": boundary_point, "b": region}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::Touches,
                json!({"a": square(0.0, 0.0, 1.0, 1.0), "b": square(1.0, 0.0, 2.0, 1.0)}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::Overlaps,
                json!({"a": square(0.0, 0.0, 2.0, 2.0), "b": square(1.0, 1.0, 3.0, 3.0)}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::Disjoint,
                json!({"a": point(0.0, 0.0), "b": point(1.0, 1.0)}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::TopologicallyEquals,
                json!({
                    "a": {"type": "LineString", "coordinates": [[0.0, 0.0], [1.0, 1.0]]},
                    "b": {"type": "LineString", "coordinates": [[1.0, 1.0], [0.0, 0.0]]}
                }),
                "result"
            ),
            json!(true)
        );
    }

    #[test]
    fn crosses_d_within_and_raw_relate_pattern_are_available() {
        let horizontal = json!({"type": "LineString", "coordinates": [[0.0, 1.0], [2.0, 1.0]]});
        let vertical = json!({"type": "LineString", "coordinates": [[1.0, 0.0], [1.0, 2.0]]});
        assert_eq!(
            output(
                Operation::Crosses,
                json!({"a": horizontal, "b": vertical}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::DWithin,
                json!({"a": point(0.0, 0.0), "b": point(0.3, 0.4), "distance": 0.5}),
                "result"
            ),
            json!(true)
        );
        assert_eq!(
            output(
                Operation::RelatePattern,
                json!({"a": point(1.0, 1.0), "b": point(1.0, 1.0), "pattern": "T********"}),
                "result"
            ),
            json!(true)
        );
        assert!(
            execute(
                Operation::RelatePattern,
                &json!({"a": point(1.0, 1.0), "b": point(1.0, 1.0), "pattern": "bad"})
            )
            .is_err()
        );
    }

    #[test]
    fn validity_nodes_report_invalid_topology_without_rejecting_the_value() {
        let bow_tie = json!({
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0], [0.0, 0.0]]]
        });
        assert_eq!(
            output(
                Operation::IsTopologicallyValid,
                json!({"geometry": bow_tie}),
                "result"
            ),
            json!(false)
        );
        assert!(
            output(
                Operation::ValidityReason,
                json!({"geometry": bow_tie}),
                "reason"
            )
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
        );
        assert!(
            !output(
                Operation::ValidationErrors,
                json!({"geometry": bow_tie}),
                "errors"
            )
            .as_array()
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn validity_nodes_error_when_topology_work_exceeds_the_budget() {
        let mut ring = (0..1_415)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64 / 1_415.0;
                json!([angle.cos(), angle.sin()])
            })
            .collect::<Vec<_>>();
        ring.push(ring[0].clone());
        let geometry = json!({"type":"Polygon", "coordinates":[ring]});

        for operation in [
            Operation::IsTopologicallyValid,
            Operation::ValidityReason,
            Operation::ValidationErrors,
        ] {
            assert!(
                execute(operation, &json!({"geometry":geometry.clone()}))
                    .unwrap_err()
                    .to_string()
                    .contains("topology validation exceeds the work limit")
            );
        }
    }

    #[test]
    fn repair_removes_redundant_positions_recursively_and_rejects_collapsed_lines() {
        let repeated = json!({
            "type": "Polygon",
            "coordinates": [[
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 1.0],
                [0.0, 0.0]
            ]]
        });
        let repaired = output(
            Operation::Repair,
            json!({
                "geometry": {
                    "type": "GeometryCollection",
                    "geometries": [repeated]
                }
            }),
            "geometry_out",
        );
        assert!(to_geo(&repaired).unwrap().is_valid());
        assert_eq!(
            repaired["geometries"][0]["coordinates"][0]
                .as_array()
                .unwrap()
                .len(),
            5
        );

        assert!(
            execute(
                Operation::Repair,
                &json!({
                    "geometry": {
                        "type": "LineString",
                        "coordinates": [[0.0, 0.0], [0.0, 0.0]]
                    }
                })
            )
            .is_err()
        );
    }

    #[test]
    fn repair_budget_counts_segments_after_adjacent_duplicates_are_removed() {
        let mut ring = Vec::with_capacity(40_001);
        for position in [
            json!([0.0, 0.0]),
            json!([1.0, 0.0]),
            json!([1.0, 1.0]),
            json!([0.0, 1.0]),
        ] {
            ring.extend(std::iter::repeat_n(position, 10_000));
        }
        ring.push(json!([0.0, 0.0]));
        let repaired = output(
            Operation::Repair,
            json!({
                "geometry":{"type":"Polygon", "coordinates":[ring]}
            }),
            "geometry_out",
        );
        assert_eq!(repaired["coordinates"][0].as_array().unwrap().len(), 5);
    }

    #[test]
    fn repair_helpers_remain_bounded_at_the_position_limit() {
        let mut ring = (0..99_999)
            .map(|index| Coord {
                x: index as f64 / 1_000.0,
                y: (index % 997) as f64 / 1_000.0,
            })
            .collect::<Vec<_>>();
        ring.push(ring[0]);
        assert_eq!(
            clean_line(LineString::new(ring), true).unwrap().0.len(),
            100_000
        );

        let points = (0..100_000)
            .map(|index| {
                geo::Point::new(
                    (index % 1_000) as f64 / 1_000.0,
                    (index / 1_000) as f64 / 1_000.0,
                )
            })
            .collect::<Vec<_>>();
        let Geometry::MultiPoint(repaired) =
            repair_geometry(Geometry::MultiPoint(MultiPoint(points))).unwrap()
        else {
            panic!("repair changed the geometry kind");
        };
        assert_eq!(repaired.0.len(), 100_000);
    }

    #[tokio::test]
    async fn parallel_repair_preserves_multiline_member_order() {
        const LINES: usize = 4;
        const POSITIONS: usize = 4_096;
        let lines = (0..LINES)
            .map(|line_index| {
                (0..POSITIONS)
                    .map(|position| json!([-170.0 + (position % 340) as f64, line_index as f64]))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let values = super::super::cpu::run(move || {
            execute(
                Operation::Repair,
                &json!({
                    "geometry":{"type":"MultiLineString", "coordinates":lines}
                }),
            )
        })
        .await
        .unwrap();
        let geometry = values
            .into_iter()
            .find(|(name, _)| *name == "geometry_out")
            .unwrap()
            .1;
        let repaired = geometry["coordinates"].as_array().unwrap();
        assert_eq!(repaired.len(), LINES);
        for (index, line) in repaired.iter().enumerate() {
            assert_eq!(line.as_array().unwrap().len(), POSITIONS);
            assert_eq!(line[0], json!([-170.0, index as f64]));
        }
    }

    #[test]
    fn repair_charges_collection_topology_work_before_member_repairs() {
        let polygons = [-10.0, 10.0]
            .into_iter()
            .map(|center| {
                let mut ring = (0..1_001)
                    .map(|index| {
                        let angle = std::f64::consts::TAU * index as f64 / 1_001.0;
                        json!([center + angle.cos(), angle.sin()])
                    })
                    .collect::<Vec<_>>();
                ring.push(ring[0].clone());
                json!({"type":"Polygon", "coordinates":[ring]})
            })
            .collect::<Vec<_>>();
        let error = execute(
            Operation::Repair,
            &json!({
                "geometry":{"type":"GeometryCollection", "geometries":polygons}
            }),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("topology validation exceeds the work limit")
        );
    }

    #[test]
    fn repaired_members_share_one_emitted_position_budget() {
        let emitted = AtomicUsize::new(MAX_GEOMETRY_POSITIONS - 1);
        repair_geometry_inner(Geometry::Point(geo::Point::new(0.0, 0.0)), &emitted).unwrap();
        let error = repair_geometry_inner(Geometry::Point(geo::Point::new(1.0, 1.0)), &emitted)
            .unwrap_err();
        assert!(error.to_string().contains("position limit"));
    }

    #[test]
    fn empty_and_buffer_nodes_handle_edges_and_validate_style() {
        assert_eq!(
            output(
                Operation::IsEmpty,
                json!({"geometry": {"type": "GeometryCollection", "geometries": []}}),
                "result"
            ),
            json!(true)
        );
        let buffer_inputs = json!({
            "geometry": point(10.0, 10.0),
            "distance": 0.1,
            "cap": "Round",
            "join": "Round",
            "arc_step_degrees": DEFAULT_STYLE_ANGLE_DEGREES,
            "miter_min_angle_degrees": DEFAULT_STYLE_ANGLE_DEGREES
        });
        let buffered = output(Operation::PlanarBuffer, buffer_inputs, "geometry_out");
        assert!((area(&buffered) - std::f64::consts::PI * 0.01).abs() < 0.001);

        assert!(
            execute(
                Operation::PlanarBuffer,
                &json!({
                    "geometry": point(10.0, 10.0),
                    "distance": 0.1,
                    "cap": "Round",
                    "join": "Round",
                    "arc_step_degrees": 0.0,
                    "miter_min_angle_degrees": DEFAULT_STYLE_ANGLE_DEGREES
                })
            )
            .is_err()
        );
    }

    #[test]
    fn planar_buffer_rejects_round_expansion_before_construction() {
        let coordinates = (0..1_000)
            .map(|index| {
                json!([
                    -170.0 + index as f64 / 1_000.0,
                    if index % 2 == 0 { 0.0 } else { 0.01 }
                ])
            })
            .collect::<Vec<_>>();
        let error = execute(
            Operation::PlanarBuffer,
            &json!({
                "geometry":{"type":"LineString", "coordinates":coordinates},
                "distance":0.1,
                "cap":"Round",
                "join":"Round",
                "arc_step_degrees":1.8,
                "miter_min_angle_degrees":DEFAULT_STYLE_ANGLE_DEGREES
            }),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Planar buffer may construct more than")
        );
    }

    #[test]
    fn node_definitions_expose_bounded_buffer_controls_and_subtyped_outputs() {
        let buffer = definition(Operation::PlanarBuffer);
        let arc_step = buffer
            .pins
            .values()
            .find(|pin| pin.name == "arc_step_degrees")
            .unwrap();
        assert_eq!(arc_step.options.as_ref().unwrap().range, Some((1.8, 45.0)));
        let output = buffer
            .pins
            .values()
            .find(|pin| pin.name == "geometry_out")
            .unwrap();
        assert_eq!(
            output.schema.as_deref(),
            Some(marker(GeometryKind::MultiPolygon))
        );
    }
}
