//! Planar line analysis, comparison, snapping and editing nodes.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, PinType},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    geometry::{GeometryKind, marker},
};

#[cfg(feature = "execute")]
use super::{check_topology_validation, cpu};
#[cfg(feature = "execute")]
use flow_like_geometry::{from_geo, to_geo};
#[cfg(feature = "execute")]
use flow_like_types::{
    Result, Value, anyhow, bail,
    geometry::canonicalize_geometry,
    json::{Map, json},
};
#[cfg(feature = "execute")]
use geo::{
    Closest, ClosestPoint, Coord, CoordsIter, Distance, Euclidean, Geodesic, Geometry, Intersects,
    Line, LineString, MapCoords, MultiLineString, Point,
    line_intersection::{LineIntersection, line_intersection},
    line_measures::FrechetDistance,
};
#[cfg(feature = "execute")]
use std::collections::HashMap;

#[cfg(feature = "execute")]
const MAX_PAIR_COMPARISONS: usize = 25_000_000;
#[cfg(feature = "execute")]
const MAX_GEODESIC_FRECHET_COMPARISONS: usize = 2_000_000;

#[derive(Clone, Copy, Debug)]
enum Operation {
    ShortestLine,
    HausdorffDistance,
    PlanarFrechetDistance,
    GeodesicFrechetDistance,
    SnapToGrid,
    OrientPolygonRings,
    MergeConnectedLines,
    SplitLineAtPoint,
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
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    let pin = node.add_output_pin(
        "geometry_out",
        "Geometry",
        description,
        VariableType::Geometry,
    );
    pin.schema = kind.map(|kind| marker(kind).to_string());
    pin
}

fn float_input<'a>(node: &'a mut Node, name: &str, description: &str) -> &'a mut Pin {
    node.add_input_pin(name, name, description, VariableType::Float)
}

fn distance_output(node: &mut Node, description: &str) {
    node.add_output_pin("distance", "Distance", description, VariableType::Float);
}

fn definition(operation: Operation) -> Node {
    use Operation::*;
    let (id, alias, title, description) = match operation {
        ShortestLine => (
            "geometry_shortest_line",
            "shortestLine",
            "Geometry Shortest Connection (Planar)",
            "Returns the shortest connection from A to B in coordinate degrees. Intersecting inputs return a Point; disjoint inputs return a LineString.",
        ),
        HausdorffDistance => (
            "geometry_hausdorff_distance",
            "hausdorffDistance",
            "Geometry Hausdorff Distance (Vertices)",
            "Computes symmetric planar Hausdorff distance in coordinate degrees using only stored coordinate positions. It does not measure distance between continuous segments.",
        ),
        PlanarFrechetDistance => (
            "geometry_planar_frechet_distance",
            "planarFrechetDistance",
            "LineString Fréchet Distance (Degrees)",
            "Computes discrete Fréchet distance between LineString position sequences using planar coordinate degrees.",
        ),
        GeodesicFrechetDistance => (
            "geometry_geodesic_frechet_distance",
            "geodesicFrechetDistance",
            "LineString Fréchet Distance (Meters)",
            "Computes discrete Fréchet distance between LineString position sequences using WGS 84 ellipsoidal point distances in meters.",
        ),
        SnapToGrid => (
            "geometry_snap_to_grid",
            "snapToGrid",
            "Snap Geometry to Grid",
            "Rounds every longitude and latitude to the nearest multiple of a positive grid size in degrees. Fails if coordinates leave WGS 84 bounds or topology collapses.",
        ),
        OrientPolygonRings => (
            "geometry_orient_polygon_rings",
            "orientPolygonRings",
            "Orient Polygon Rings",
            "Normalizes Polygon or MultiPolygon winding to counterclockwise exterior rings and clockwise interior rings, then validates topology.",
        ),
        MergeConnectedLines => (
            "geometry_merge_connected_lines",
            "mergeConnectedLines",
            "Merge Connected Lines",
            "Merges MultiLineString members at exactly equal endpoints. Chains stop at endpoints with a degree other than two, preserving network junctions.",
        ),
        SplitLineAtPoint => (
            "geometry_split_line_at_point",
            "splitLineAtPoint",
            "Split LineString at Point",
            "Projects a Point to the nearest planar position on a LineString and splits there when the distance is within the nonnegative tolerance in degrees. Endpoint splits return the original line as one member.",
        ),
    };

    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");

    match operation {
        ShortestLine => {
            geometry_input(&mut node, "a", "Source geometry", None);
            geometry_input(&mut node, "b", "Destination geometry", None);
            geometry_output(
                &mut node,
                "Shortest Point or LineString connection from A to B",
                None,
            );
        }
        HausdorffDistance => {
            geometry_input(&mut node, "a", "First geometry", None);
            geometry_input(&mut node, "b", "Second geometry", None);
            distance_output(&mut node, "Vertex-set Hausdorff distance in degrees");
        }
        PlanarFrechetDistance | GeodesicFrechetDistance => {
            geometry_input(
                &mut node,
                "a",
                "First LineString",
                Some(GeometryKind::LineString),
            );
            geometry_input(
                &mut node,
                "b",
                "Second LineString",
                Some(GeometryKind::LineString),
            );
            distance_output(
                &mut node,
                if matches!(operation, GeodesicFrechetDistance) {
                    "Discrete Fréchet distance in meters"
                } else {
                    "Discrete Fréchet distance in coordinate degrees"
                },
            );
        }
        SnapToGrid => {
            geometry_input(&mut node, "geometry", "Geometry to snap", None);
            float_input(
                &mut node,
                "grid_size",
                "Positive longitude and latitude grid spacing in degrees",
            );
            geometry_output(&mut node, "Snapped geometry", None);
        }
        OrientPolygonRings => {
            geometry_input(
                &mut node,
                "geometry",
                "Polygon or MultiPolygon to orient",
                None,
            );
            geometry_output(&mut node, "Oriented Polygon or MultiPolygon", None);
        }
        MergeConnectedLines => {
            geometry_input(
                &mut node,
                "geometry",
                "MultiLineString to merge",
                Some(GeometryKind::MultiLineString),
            );
            geometry_output(
                &mut node,
                "Merged line chains",
                Some(GeometryKind::MultiLineString),
            );
        }
        SplitLineAtPoint => {
            geometry_input(
                &mut node,
                "geometry",
                "LineString to split",
                Some(GeometryKind::LineString),
            );
            geometry_input(
                &mut node,
                "point",
                "Point to project onto the line",
                Some(GeometryKind::Point),
            );
            float_input(
                &mut node,
                "tolerance",
                "Maximum planar projection distance in degrees",
            )
            .set_default_value(Some(flow_like_types::json::json!(0.0)));
            geometry_output(
                &mut node,
                "One or two split line members",
                Some(GeometryKind::MultiLineString),
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
        .ok_or_else(|| anyhow!("Missing geometry input {name}"))
}

#[cfg(feature = "execute")]
fn number(inputs: &Value, name: &str) -> Result<f64> {
    let number = input(inputs, name)?
        .as_f64()
        .ok_or_else(|| anyhow!("{name} must be a finite number"))?;
    if !number.is_finite() {
        bail!("{name} must be finite");
    }
    Ok(number)
}

#[cfg(feature = "execute")]
fn checked_geometry(value: &Value) -> Result<Geometry<f64>> {
    let value = canonicalize_geometry(value, None)?;
    let geometry = to_geo(&value)?;
    check_topology_validation(&geometry, "Invalid geometry topology")?;
    Ok(geometry)
}

#[cfg(feature = "execute")]
fn checked_line(value: &Value) -> Result<LineString<f64>> {
    let geometry = checked_geometry(value)?;
    let Geometry::LineString(line) = geometry else {
        bail!("Expected LineString geometry");
    };
    Ok(line)
}

#[cfg(feature = "execute")]
fn checked_point(value: &Value) -> Result<Point<f64>> {
    let geometry = checked_geometry(value)?;
    let Geometry::Point(point) = geometry else {
        bail!("Expected Point geometry");
    };
    Ok(point)
}

#[cfg(feature = "execute")]
fn encoded_geometry(geometry: Geometry<f64>) -> Result<Value> {
    let value = canonicalize_geometry(&from_geo(&geometry)?, None)?;
    check_topology_validation(&to_geo(&value)?, "Geometry operation collapsed topology")?;
    Ok(value)
}

#[cfg(feature = "execute")]
fn finite_distance(distance: f64) -> Result<Value> {
    if !distance.is_finite() {
        bail!("Geometry distance is undefined for these inputs");
    }
    Ok(json!(distance))
}

#[cfg(feature = "execute")]
fn guard_pair_comparisons(a: usize, b: usize, operation: &str) -> Result<()> {
    guard_pair_comparisons_with_limit(a, b, operation, MAX_PAIR_COMPARISONS)
}

#[cfg(feature = "execute")]
fn guard_pair_comparisons_with_limit(
    a: usize,
    b: usize,
    operation: &str,
    limit: usize,
) -> Result<()> {
    let comparisons = a.checked_mul(b).unwrap_or(usize::MAX);
    if comparisons > limit {
        bail!(
            "{operation} requires {comparisons} coordinate-pair comparisons, exceeding the limit of {limit}"
        );
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn collect_segments(geometry: &Geometry<f64>, output: &mut Vec<Line<f64>>) {
    match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => {}
        Geometry::LineString(line) => output.extend(line.lines()),
        Geometry::MultiLineString(lines) => {
            for line in &lines.0 {
                output.extend(line.lines());
            }
        }
        Geometry::Polygon(polygon) => {
            output.extend(polygon.exterior().lines());
            for ring in polygon.interiors() {
                output.extend(ring.lines());
            }
        }
        Geometry::MultiPolygon(polygons) => {
            for polygon in &polygons.0 {
                output.extend(polygon.exterior().lines());
                for ring in polygon.interiors() {
                    output.extend(ring.lines());
                }
            }
        }
        Geometry::GeometryCollection(collection) => {
            for geometry in &collection.0 {
                collect_segments(geometry, output);
            }
        }
        _ => unreachable!("GeoJSON conversion only produces supported geometry variants"),
    }
}

#[cfg(feature = "execute")]
fn closest_coord(geometry: &Geometry<f64>, coord: Coord<f64>) -> Option<(Coord<f64>, bool)> {
    match geometry.closest_point(&Point(coord)) {
        Closest::Intersection(point) => Some((point.0, true)),
        Closest::SinglePoint(point) => Some((point.0, false)),
        Closest::Indeterminate => None,
    }
}

#[cfg(feature = "execute")]
fn squared_distance(a: Coord<f64>, b: Coord<f64>) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

#[cfg(feature = "execute")]
fn coordinate_pair_is_before(
    candidate_from: Coord<f64>,
    candidate_to: Coord<f64>,
    current_from: Coord<f64>,
    current_to: Coord<f64>,
) -> bool {
    [
        candidate_from.x.total_cmp(&current_from.x),
        candidate_from.y.total_cmp(&current_from.y),
        candidate_to.x.total_cmp(&current_to.x),
        candidate_to.y.total_cmp(&current_to.y),
    ]
    .into_iter()
    .find(|ordering| !ordering.is_eq())
    .is_some_and(|ordering| ordering.is_lt())
}

#[cfg(feature = "execute")]
fn shortest_line(a: &Geometry<f64>, b: &Geometry<f64>) -> Result<(Coord<f64>, Coord<f64>)> {
    let a_coords: Vec<_> = a.coords_iter().collect();
    let b_coords: Vec<_> = b.coords_iter().collect();
    if a_coords.is_empty() || b_coords.is_empty() {
        bail!("Empty geometries have no shortest line");
    }
    guard_pair_comparisons(a_coords.len(), b_coords.len(), "Shortest Line")?;
    let work = a_coords.len().saturating_mul(b_coords.len());

    if a.intersects(b) {
        // Ordered search preserves the first common position and stops unnecessary work.
        let common_from_a = cpu::find_map_first_ordered(&a_coords, work, |_, coord| {
            closest_coord(b, *coord).and_then(|(point, intersects)| intersects.then_some(point))
        });
        if let Some(point) = common_from_a {
            return Ok((point, point));
        }
        let common_from_b = cpu::find_map_first_ordered(&b_coords, work, |_, coord| {
            closest_coord(a, *coord).and_then(|(point, intersects)| intersects.then_some(point))
        });
        if let Some(point) = common_from_b {
            return Ok((point, point));
        }
        let mut a_segments = Vec::new();
        let mut b_segments = Vec::new();
        collect_segments(a, &mut a_segments);
        collect_segments(b, &mut b_segments);
        guard_pair_comparisons(a_segments.len(), b_segments.len(), "Shortest Line")?;
        let common_segments = cpu::find_map_first_ordered(
            &a_segments,
            a_segments.len().saturating_mul(b_segments.len()),
            |_, a_segment| {
                b_segments.iter().find_map(|b_segment| {
                    line_intersection(*a_segment, *b_segment).map(|intersection| match intersection
                    {
                        LineIntersection::SinglePoint { intersection, .. } => intersection,
                        LineIntersection::Collinear { intersection } => intersection.start,
                    })
                })
            },
        );
        if let Some(point) = common_segments {
            return Ok((point, point));
        }
        bail!("Intersecting geometries did not expose a finite common position");
    }

    let mut best: Option<(Coord<f64>, Coord<f64>, f64)> = None;
    let mut consider = |from: Coord<f64>, to: Coord<f64>| {
        let distance = squared_distance(from, to);
        if best.is_none_or(|(current_from, current_to, current_distance)| {
            distance < current_distance
                || (distance == current_distance
                    && coordinate_pair_is_before(from, to, current_from, current_to))
        }) {
            best = Some((from, to, distance));
        }
    };
    let candidates = cpu::map_indices(
        a_coords.len() + b_coords.len(),
        work.saturating_mul(2),
        |index| {
            if let Some(coord) = a_coords.get(index) {
                closest_coord(b, *coord).map(|(closest, _)| (*coord, closest))
            } else {
                let coord = b_coords[index - a_coords.len()];
                closest_coord(a, coord).map(|(closest, _)| (closest, coord))
            }
        },
    );
    for (from, to) in candidates.into_iter().flatten() {
        consider(from, to);
    }
    best.map(|(from, to, _)| (from, to))
        .ok_or_else(|| anyhow!("Could not determine a shortest line"))
}

#[cfg(feature = "execute")]
fn vertex_hausdorff_distance(a: &[Coord<f64>], b: &[Coord<f64>]) -> f64 {
    let nearest = cpu::map_indices(
        a.len() + b.len(),
        a.len().saturating_mul(b.len()).saturating_mul(2),
        |index| {
            let (coord, other) = if let Some(coord) = a.get(index) {
                (*coord, b)
            } else {
                (b[index - a.len()], a)
            };
            other
                .iter()
                .map(|other| Euclidean.distance(coord, *other))
                .fold(f64::MAX, f64::min)
        },
    );
    nearest.into_iter().fold(f64::MIN, f64::max)
}

#[cfg(feature = "execute")]
type CoordKey = (u64, u64);

#[cfg(feature = "execute")]
fn coordinate_key(coord: Coord<f64>) -> CoordKey {
    let bits = |value: f64| if value == 0.0 { 0.0 } else { value }.to_bits();
    (bits(coord.x), bits(coord.y))
}

#[cfg(feature = "execute")]
fn merge_connected_lines(lines: MultiLineString<f64>) -> MultiLineString<f64> {
    let lines: Vec<Vec<Coord<f64>>> = lines.0.into_iter().map(|line| line.0).collect();
    let mut adjacency: HashMap<CoordKey, Vec<(usize, bool)>> = HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        adjacency
            .entry(coordinate_key(line[0]))
            .or_default()
            .push((index, true));
        adjacency
            .entry(coordinate_key(*line.last().unwrap()))
            .or_default()
            .push((index, false));
    }

    let mut visited = vec![false; lines.len()];
    let mut merged = Vec::new();
    for boundary_phase in [true, false] {
        for seed in 0..lines.len() {
            if visited[seed] {
                continue;
            }
            let start_degree = adjacency[&coordinate_key(lines[seed][0])].len();
            let end_degree = adjacency[&coordinate_key(*lines[seed].last().unwrap())].len();
            let touches_boundary = start_degree != 2 || end_degree != 2;
            if touches_boundary != boundary_phase {
                continue;
            }

            let mut chain = lines[seed].clone();
            if start_degree == 2 && end_degree != 2 {
                chain.reverse();
            }
            visited[seed] = true;

            loop {
                let end_key = coordinate_key(*chain.last().unwrap());
                let incidents = &adjacency[&end_key];
                if incidents.len() != 2 {
                    break;
                }
                let Some(&(next_index, matches_start)) =
                    incidents.iter().find(|(index, _)| !visited[*index])
                else {
                    break;
                };
                let mut next = lines[next_index].clone();
                if !matches_start {
                    next.reverse();
                }
                chain.extend(next.into_iter().skip(1));
                visited[next_index] = true;
            }
            merged.push(LineString(chain));
        }
    }
    MultiLineString(merged)
}

#[cfg(feature = "execute")]
fn project_to_segment(point: Coord<f64>, segment: Line<f64>) -> Option<(f64, Coord<f64>, f64)> {
    let dx = segment.end.x - segment.start.x;
    let dy = segment.end.y - segment.start.y;
    let denominator = dx * dx + dy * dy;
    if denominator == 0.0 {
        return None;
    }
    let raw = ((point.x - segment.start.x) * dx + (point.y - segment.start.y) * dy) / denominator;
    let t = raw.clamp(0.0, 1.0);
    let projected = if t == 0.0 {
        segment.start
    } else if t == 1.0 {
        segment.end
    } else {
        Coord {
            x: segment.start.x + t * dx,
            y: segment.start.y + t * dy,
        }
    };
    Some((t, projected, squared_distance(point, projected)))
}

#[cfg(feature = "execute")]
fn split_line_at_point(
    mut line: LineString<f64>,
    point: Point<f64>,
    tolerance: f64,
) -> Result<MultiLineString<f64>> {
    line.0
        .dedup_by(|a, b| coordinate_key(*a) == coordinate_key(*b));
    let mut best: Option<(usize, f64, Coord<f64>, f64)> = None;
    for (index, segment) in line.lines().enumerate() {
        let Some((t, projected, distance)) = project_to_segment(point.0, segment) else {
            continue;
        };
        if best.is_none_or(|(_, _, _, current)| distance < current) {
            best = Some((index, t, projected, distance));
        }
    }
    let (segment_index, t, projected, distance_squared) =
        best.ok_or_else(|| anyhow!("LineString has no nonzero segment"))?;
    let distance = distance_squared.sqrt();
    if distance > tolerance {
        bail!("Point is {distance} degrees from the LineString, beyond tolerance {tolerance}");
    }

    if (segment_index == 0 && t == 0.0) || (segment_index + 1 == line.0.len() - 1 && t == 1.0) {
        return Ok(MultiLineString(vec![line]));
    }

    let (left, right) = if t == 0.0 {
        let split = segment_index;
        (line.0[..=split].to_vec(), line.0[split..].to_vec())
    } else if t == 1.0 {
        let split = segment_index + 1;
        (line.0[..=split].to_vec(), line.0[split..].to_vec())
    } else {
        let mut left = line.0[..=segment_index].to_vec();
        left.push(projected);
        let mut right = vec![projected];
        right.extend_from_slice(&line.0[segment_index + 1..]);
        (left, right)
    };
    Ok(MultiLineString(vec![LineString(left), LineString(right)]))
}

#[cfg(feature = "execute")]
fn execute(operation: Operation, inputs: &Value) -> Result<Vec<(&'static str, Value)>> {
    use Operation::*;
    match operation {
        ShortestLine => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            let (from, to) = shortest_line(&a, &b)?;
            let result = if from == to {
                Geometry::Point(Point::from(from))
            } else {
                Geometry::LineString(LineString::new(vec![from, to]))
            };
            Ok(vec![("geometry_out", encoded_geometry(result)?)])
        }
        HausdorffDistance => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            let a_count = a.coords_count();
            let b_count = b.coords_count();
            if a_count == 0 || b_count == 0 {
                bail!("Empty geometries have no Hausdorff distance");
            }
            guard_pair_comparisons(a_count, b_count, "Hausdorff Distance")?;
            let a_coords: Vec<_> = a.coords_iter().collect();
            let b_coords: Vec<_> = b.coords_iter().collect();
            Ok(vec![(
                "distance",
                finite_distance(vertex_hausdorff_distance(&a_coords, &b_coords))?,
            )])
        }
        PlanarFrechetDistance | GeodesicFrechetDistance => {
            let a = checked_line(input(inputs, "a")?)?;
            let b = checked_line(input(inputs, "b")?)?;
            let geodesic = matches!(operation, GeodesicFrechetDistance);
            if geodesic {
                guard_pair_comparisons_with_limit(
                    a.0.len(),
                    b.0.len(),
                    "Geodesic Fréchet Distance",
                    MAX_GEODESIC_FRECHET_COMPARISONS,
                )?;
            } else {
                guard_pair_comparisons(a.0.len(), b.0.len(), "Planar Fréchet Distance")?;
            }
            let distance = if geodesic {
                Geodesic.frechet_distance(&a, &b)
            } else {
                Euclidean.frechet_distance(&a, &b)
            };
            Ok(vec![("distance", finite_distance(distance)?)])
        }
        SnapToGrid => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            let grid_size = number(inputs, "grid_size")?;
            if grid_size <= 0.0 {
                bail!("grid_size must be greater than zero degrees");
            }
            let snap = |value: f64| {
                let snapped = (value / grid_size).round() * grid_size;
                if snapped == 0.0 { 0.0 } else { snapped }
            };
            let snapped = geometry.map_coords(|coord| Coord {
                x: snap(coord.x),
                y: snap(coord.y),
            });
            Ok(vec![("geometry_out", encoded_geometry(snapped)?)])
        }
        OrientPolygonRings => {
            let value = input(inputs, "geometry")?;
            let kind = value.get("type").and_then(Value::as_str);
            if !matches!(kind, Some("Polygon" | "MultiPolygon")) {
                bail!("Orient Polygon Rings requires Polygon or MultiPolygon");
            }
            let geometry = checked_geometry(value)?;
            Ok(vec![("geometry_out", encoded_geometry(geometry)?)])
        }
        MergeConnectedLines => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            let Geometry::MultiLineString(lines) = geometry else {
                bail!("Merge Connected Lines requires MultiLineString");
            };
            let merged = merge_connected_lines(lines);
            Ok(vec![(
                "geometry_out",
                encoded_geometry(Geometry::MultiLineString(merged))?,
            )])
        }
        SplitLineAtPoint => {
            let line = checked_line(input(inputs, "geometry")?)?;
            let point = checked_point(input(inputs, "point")?)?;
            let tolerance = number(inputs, "tolerance")?;
            if tolerance < 0.0 {
                bail!("tolerance must be nonnegative degrees");
            }
            let result = split_line_at_point(line, point, tolerance)?;
            Ok(vec![(
                "geometry_out",
                encoded_geometry(Geometry::MultiLineString(result))?,
            )])
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
pub struct GeometryShortestLineNode;
implement_node!(GeometryShortestLineNode, Operation::ShortestLine);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryHausdorffDistanceNode;
implement_node!(GeometryHausdorffDistanceNode, Operation::HausdorffDistance);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPlanarFrechetDistanceNode;
implement_node!(
    GeometryPlanarFrechetDistanceNode,
    Operation::PlanarFrechetDistance
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicFrechetDistanceNode;
implement_node!(
    GeometryGeodesicFrechetDistanceNode,
    Operation::GeodesicFrechetDistance
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySnapToGridNode;
implement_node!(GeometrySnapToGridNode, Operation::SnapToGrid);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryOrientPolygonRingsNode;
implement_node!(
    GeometryOrientPolygonRingsNode,
    Operation::OrientPolygonRings
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMergeConnectedLinesNode;
implement_node!(
    GeometryMergeConnectedLinesNode,
    Operation::MergeConnectedLines
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySplitLineAtPointNode;
implement_node!(GeometrySplitLineAtPointNode, Operation::SplitLineAtPoint);

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
    fn line_specific_nodes_publish_exact_subtypes() {
        let split = definition(Operation::SplitLineAtPoint);
        assert_eq!(
            pin(&split, "geometry").schema.as_deref(),
            Some(marker(GeometryKind::LineString))
        );
        assert_eq!(
            pin(&split, "point").schema.as_deref(),
            Some(marker(GeometryKind::Point))
        );
        assert_eq!(
            pin(&split, "geometry_out").schema.as_deref(),
            Some(marker(GeometryKind::MultiLineString))
        );
        assert_eq!(
            pin(&split, "tolerance").default_value,
            Some(flow_like_types::json::to_vec(&flow_like_types::json::json!(0.0)).unwrap())
        );
    }

    #[test]
    fn dynamic_geometry_nodes_keep_open_subtypes() {
        for operation in [Operation::SnapToGrid, Operation::OrientPolygonRings] {
            let node = definition(operation);
            assert_eq!(pin(&node, "geometry").schema, None);
            assert_eq!(pin(&node, "geometry_out").schema, None);
        }
    }
}

#[cfg(all(test, feature = "execute"))]
mod execution_tests {
    use super::*;
    use geo::HausdorffDistance;

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
    fn shortest_line_handles_crossings_and_disjoint_segments() {
        let crossing = result(
            Operation::ShortestLine,
            json!({
                "a":line(json!([[0,0],[2,2]])),
                "b":line(json!([[0,2],[2,0]]))
            }),
            "geometry_out",
        );
        assert_eq!(crossing, point(1.0, 1.0));

        let disjoint = result(
            Operation::ShortestLine,
            json!({
                "a":line(json!([[0,0],[2,0]])),
                "b":line(json!([[1,2],[3,2]]))
            }),
            "geometry_out",
        );
        assert_eq!(disjoint["coordinates"], json!([[1.0, 0.0], [1.0, 2.0]]));
    }

    #[tokio::test]
    async fn parallel_shortest_connections_preserve_ties_and_first_common_positions() {
        let descending_line = |y: f64| {
            line(json!(
                (0..192)
                    .map(|index| [10.0 - index as f64 / 191.0 * 20.0, y])
                    .collect::<Vec<_>>()
            ))
        };
        let crossing_lines = |horizontal: bool| {
            json!({
                "type":"MultiLineString",
                "coordinates":(0..128).map(|index| {
                    let position = 5.0 - index as f64 / 127.0 * 10.0;
                    if horizontal {
                        [[10.0,position],[-10.0,position]]
                    } else {
                        [[position,10.0],[position,-10.0]]
                    }
                }).collect::<Vec<_>>()
            })
        };
        for (inputs, expected) in [
            (
                json!({"a":descending_line(0.0),"b":descending_line(2.0)}),
                line(json!([[-10.0, 0.0], [-10.0, 2.0]])),
            ),
            (
                json!({"a":descending_line(0.0),"b":descending_line(0.0)}),
                point(10.0, 0.0),
            ),
            (
                json!({"a":crossing_lines(true),"b":crossing_lines(false)}),
                point(5.0, 5.0),
            ),
        ] {
            let serial = execute(Operation::ShortestLine, &inputs).unwrap();
            assert_eq!(serial[0].1, expected);
            let parallel = cpu::run(move || execute(Operation::ShortestLine, &inputs))
                .await
                .unwrap();
            assert_eq!(parallel, serial);
        }
    }

    #[tokio::test]
    async fn parallel_hausdorff_matches_geo_vertex_distance_exactly() {
        let inputs = json!({
            "a":{
                "type":"GeometryCollection",
                "geometries":[
                    {"type":"MultiPoint","coordinates":(0..192).map(|index| {
                        [index as f64 / 10.0,(index * 17 % 193) as f64 / 5.0]
                    }).collect::<Vec<_>>()},
                    line(json!([[0.0,0.0],[10.0,10.0]]))
                ]
            },
            "b":{
                "type":"MultiPoint",
                "coordinates":(0..173).map(|index| {
                    [index as f64 / 11.0,(index * 29 % 179) as f64 / 7.0]
                }).collect::<Vec<_>>()
            }
        });
        let a = checked_geometry(&inputs["a"]).unwrap();
        let b = checked_geometry(&inputs["b"]).unwrap();
        let expected = a.hausdorff_distance(&b);
        let serial = execute(Operation::HausdorffDistance, &inputs).unwrap();
        assert_eq!(serial[0].1, json!(expected));
        let parallel = cpu::run(move || execute(Operation::HausdorffDistance, &inputs))
            .await
            .unwrap();
        assert_eq!(parallel, serial);
    }

    #[tokio::test]
    async fn parallel_distance_operations_preserve_empty_and_budget_errors() {
        let multipoint = |count: usize, y: f64| {
            json!({
                "type":"MultiPoint",
                "coordinates":(0..count).map(|index| {
                    [index as f64 / count as f64,y]
                }).collect::<Vec<_>>()
            })
        };
        for operation in [Operation::ShortestLine, Operation::HausdorffDistance] {
            for (inputs, message) in [
                (
                    json!({"a":multipoint(0,0.0),"b":point(0.0,0.0)}),
                    "Empty geometries",
                ),
                (
                    json!({"a":multipoint(5_001,0.0),"b":multipoint(5_000,1.0)}),
                    "exceeding the limit of 25000000",
                ),
            ] {
                let serial = execute(operation, &inputs).unwrap_err().to_string();
                assert!(serial.contains(message));
                let parallel = cpu::run(move || execute(operation, &inputs))
                    .await
                    .unwrap_err()
                    .to_string();
                assert_eq!(parallel, serial);
            }
        }
    }

    #[test]
    fn hausdorff_is_explicitly_a_vertex_set_measure() {
        let distance = result(
            Operation::HausdorffDistance,
            json!({
                "a":line(json!([[0,0],[10,0]])),
                "b":line(json!([[0,0],[5,0],[10,0]]))
            }),
            "distance",
        );
        assert_eq!(distance, 5.0);
        assert!(
            execute(
                Operation::HausdorffDistance,
                &json!({
                    "a":{"type":"MultiPoint","coordinates":[]},
                    "b":point(0.0,0.0)
                })
            )
            .is_err()
        );
    }

    #[test]
    fn discrete_frechet_supports_planar_and_wgs84_metrics() {
        let inputs = json!({
            "a":line(json!([[0,0],[1,0]])),
            "b":line(json!([[0,1],[1,1]]))
        });
        assert_eq!(
            result(Operation::PlanarFrechetDistance, inputs.clone(), "distance"),
            1.0
        );
        let meters = result(Operation::GeodesicFrechetDistance, inputs, "distance")
            .as_f64()
            .unwrap();
        assert!((110_000.0..112_000.0).contains(&meters));
    }

    #[test]
    fn geodesic_frechet_rejects_expensive_coordinate_products() {
        let long_line = |position_count: usize| {
            line(Value::Array(
                (0..position_count)
                    .map(|index| json!([index as f64 / position_count as f64, 0.0]))
                    .collect(),
            ))
        };
        let error = execute(
            Operation::GeodesicFrechetDistance,
            &json!({"a":long_line(1_415),"b":long_line(1_414)}),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Geodesic Fréchet Distance"));
        assert!(error.contains("exceeding the limit of 2000000"));
    }

    #[test]
    fn snap_to_grid_rejects_collapsed_topology() {
        let snapped = result(
            Operation::SnapToGrid,
            json!({
                "geometry":line(json!([[0.1,0.1],[1.1,0.1]])),
                "grid_size":1.0
            }),
            "geometry_out",
        );
        assert_eq!(snapped["coordinates"], json!([[0.0, 0.0], [1.0, 0.0]]));
        assert!(
            execute(
                Operation::SnapToGrid,
                &json!({
                    "geometry":line(json!([[0.1,0.1],[0.2,0.2]])),
                    "grid_size":1.0
                })
            )
            .is_err()
        );
    }

    #[test]
    fn polygon_orientation_is_normalized_and_validated() {
        let oriented = result(
            Operation::OrientPolygonRings,
            json!({"geometry":{
                "type":"Polygon",
                "coordinates":[[[0,0],[0,2],[2,2],[2,0],[0,0]]]
            }}),
            "geometry_out",
        );
        let ring = oriented["coordinates"][0].as_array().unwrap();
        let area: f64 = ring
            .windows(2)
            .map(|pair| {
                pair[0][0].as_f64().unwrap() * pair[1][1].as_f64().unwrap()
                    - pair[1][0].as_f64().unwrap() * pair[0][1].as_f64().unwrap()
            })
            .sum();
        assert!(area > 0.0);
    }

    #[test]
    fn merge_connected_lines_builds_chains_and_stops_at_branches() {
        let merged = result(
            Operation::MergeConnectedLines,
            json!({"geometry":{
                "type":"MultiLineString",
                "coordinates":[[[1,0],[0,0]],[[1,0],[2,0]],[[3,0],[2,0]]]
            }}),
            "geometry_out",
        );
        assert_eq!(merged["coordinates"].as_array().unwrap().len(), 1);
        assert_eq!(
            merged["coordinates"][0],
            json!([[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]])
        );

        let branched = result(
            Operation::MergeConnectedLines,
            json!({"geometry":{
                "type":"MultiLineString",
                "coordinates":[[[0,0],[1,0]],[[1,0],[2,0]],[[1,0],[1,1]]]
            }}),
            "geometry_out",
        );
        assert_eq!(branched["coordinates"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn split_line_projects_within_tolerance() {
        let split = result(
            Operation::SplitLineAtPoint,
            json!({
                "geometry":line(json!([[0,0],[2,0]])),
                "point":point(1.0,1.0),
                "tolerance":1.0
            }),
            "geometry_out",
        );
        assert_eq!(split["coordinates"].as_array().unwrap().len(), 2);
        assert_eq!(split["coordinates"][0], json!([[0.0, 0.0], [1.0, 0.0]]));
        assert_eq!(split["coordinates"][1], json!([[1.0, 0.0], [2.0, 0.0]]));
        assert!(
            execute(
                Operation::SplitLineAtPoint,
                &json!({
                    "geometry":line(json!([[0,0],[2,0]])),
                    "point":point(1.0,1.0),
                    "tolerance":0.5
                })
            )
            .is_err()
        );
    }
}
