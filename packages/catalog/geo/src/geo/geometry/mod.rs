//! Native geometry nodes with explicit adapters for existing geo payloads.

pub mod advanced;
pub mod construct_decompose;
pub mod coordinate_systems;
#[cfg(feature = "execute")]
mod cpu;
pub mod integrations;
pub mod legacy;
pub mod linear_analysis;
pub mod nodes;
#[cfg(feature = "execute")]
mod operations;
pub mod topology;

pub use nodes::*;

#[cfg(feature = "execute")]
use flow_like_types::{
    Result, Value, anyhow, bail,
    geometry::{MAX_GEOMETRY_DEPTH, MAX_GEOMETRY_MEMBERS, MAX_GEOMETRY_POSITIONS},
};
#[cfg(feature = "execute")]
use geo::{CoordsIter, Geometry, MultiPolygon, Polygon, Validation};

/// `geo::Validation` compares every segment in a ring with every other segment,
/// then relates every hole and MultiPolygon member pair. One work unit here is
/// one potential segment-pair check. The cap deliberately overestimates indexed
/// relation work so valid inputs cannot hide unbounded quadratic validation.
#[cfg(feature = "execute")]
const MAX_TOPOLOGY_VALIDATION_WORK: usize = 2_000_000;
#[cfg(feature = "execute")]
const MAX_TOPOLOGY_VALIDATION_ERRORS: usize = 256;

/// Counts position-shaped coordinate arrays without trusting the value's shape.
/// Collection callers use this before parallel validation so their combined
/// intermediate data remains subject to the Geometry resource limits.
#[cfg(feature = "execute")]
pub(super) fn add_estimated_geometry_positions(
    value: &Value,
    positions: &mut usize,
    members: &mut usize,
    context: &str,
) -> Result<()> {
    let mut stack = vec![(value, 0usize, false)];
    while let Some((value, depth, coordinates)) = stack.pop() {
        *members = members.saturating_add(1);
        if depth > MAX_GEOMETRY_DEPTH || *members > MAX_GEOMETRY_MEMBERS {
            bail!("{context} exceeds the Geometry depth or member limit");
        }
        if coordinates {
            let Some(values) = value.as_array() else {
                continue;
            };
            if values.len() == 2 && values[0].is_number() && values[1].is_number() {
                *positions = positions.saturating_add(1);
                if *positions > MAX_GEOMETRY_POSITIONS {
                    bail!(
                        "{context} exceeds the Geometry position limit of {MAX_GEOMETRY_POSITIONS}"
                    );
                }
            } else {
                if values.len()
                    > MAX_GEOMETRY_MEMBERS
                        .saturating_sub(*members)
                        .saturating_sub(stack.len())
                {
                    bail!("{context} exceeds the Geometry depth or member limit");
                }
                stack.extend(values.iter().rev().map(|value| (value, depth + 1, true)));
            }
        } else if value.get("type").and_then(Value::as_str) == Some("GeometryCollection") {
            if let Some(geometries) = value.get("geometries").and_then(Value::as_array) {
                if geometries.len()
                    > MAX_GEOMETRY_MEMBERS
                        .saturating_sub(*members)
                        .saturating_sub(stack.len())
                {
                    bail!("{context} exceeds the Geometry depth or member limit");
                }
                stack.extend(
                    geometries
                        .iter()
                        .rev()
                        .map(|value| (value, depth + 1, false)),
                );
            }
        } else if let Some(coordinates) = value.get("coordinates") {
            if *members >= MAX_GEOMETRY_MEMBERS.saturating_sub(stack.len()) {
                bail!("{context} exceeds the Geometry depth or member limit");
            }
            stack.push((coordinates, depth + 1, true));
        }
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn charge_topology_validation_work(
    work: &mut usize,
    left_segments: usize,
    right_segments: usize,
) -> Result<()> {
    let remaining = MAX_TOPOLOGY_VALIDATION_WORK.saturating_sub(*work);
    if left_segments != 0 && right_segments > remaining / left_segments {
        bail!(
            "Geometry topology validation exceeds the work limit of {MAX_TOPOLOGY_VALIDATION_WORK} potential segment-pair checks; simplify polygon rings or reduce hole and polygon counts"
        );
    }
    *work += left_segments * right_segments;
    Ok(())
}

#[cfg(feature = "execute")]
fn ring_segment_count(ring: &geo::LineString<f64>) -> usize {
    ring.0.len().saturating_sub(1)
}

#[cfg(feature = "execute")]
fn charge_polygon_validation_work(work: &mut usize, polygon: &Polygon<f64>) -> Result<usize> {
    let exterior_segments = ring_segment_count(polygon.exterior());
    charge_topology_validation_work(work, exterior_segments, exterior_segments)?;

    let mut preceding_hole_segments = 0usize;
    let mut polygon_segments = exterior_segments;
    for hole in polygon.interiors() {
        let hole_segments = ring_segment_count(hole);
        charge_topology_validation_work(work, hole_segments, hole_segments)?;
        charge_topology_validation_work(work, exterior_segments, hole_segments)?;
        charge_topology_validation_work(work, preceding_hole_segments, hole_segments)?;
        preceding_hole_segments = preceding_hole_segments.saturating_add(hole_segments);
        polygon_segments = polygon_segments.saturating_add(hole_segments);
    }
    Ok(polygon_segments)
}

#[cfg(feature = "execute")]
fn charge_multipolygon_validation_work(
    work: &mut usize,
    polygons: &MultiPolygon<f64>,
) -> Result<()> {
    let mut preceding_polygon_segments = 0usize;
    for polygon in &polygons.0 {
        let polygon_segments = charge_polygon_validation_work(work, polygon)?;
        charge_topology_validation_work(work, preceding_polygon_segments, polygon_segments)?;
        preceding_polygon_segments = preceding_polygon_segments.saturating_add(polygon_segments);
    }
    Ok(())
}

#[cfg(feature = "execute")]
pub(super) trait BoundedTopologyValidation: Validation {
    fn charge_validation_work(&self, work: &mut usize) -> Result<()>;
}

#[cfg(feature = "execute")]
impl BoundedTopologyValidation for Polygon<f64> {
    fn charge_validation_work(&self, work: &mut usize) -> Result<()> {
        charge_polygon_validation_work(work, self).map(|_| ())
    }
}

#[cfg(feature = "execute")]
impl BoundedTopologyValidation for MultiPolygon<f64> {
    fn charge_validation_work(&self, work: &mut usize) -> Result<()> {
        charge_multipolygon_validation_work(work, self)
    }
}

#[cfg(feature = "execute")]
impl BoundedTopologyValidation for Geometry<f64> {
    fn charge_validation_work(&self, work: &mut usize) -> Result<()> {
        match self {
            Geometry::Polygon(polygon) => polygon.charge_validation_work(work),
            Geometry::MultiPolygon(polygons) => polygons.charge_validation_work(work),
            Geometry::GeometryCollection(collection) => {
                for geometry in &collection.0 {
                    geometry.charge_validation_work(work)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(feature = "execute")]
pub(super) fn ensure_topology_validation_budget<T>(geometry: &T) -> Result<()>
where
    T: BoundedTopologyValidation,
{
    let mut work = 0;
    geometry.charge_validation_work(&mut work)
}

#[cfg(feature = "execute")]
pub(super) fn check_topology_validation<T>(geometry: &T, invalid_context: &str) -> Result<()>
where
    T: BoundedTopologyValidation,
{
    if let Some(error) = topology_validation_error(geometry)? {
        return Err(anyhow!("{invalid_context}: {error}"));
    }
    Ok(())
}

#[cfg(feature = "execute")]
pub(super) fn topology_validation_error<T>(geometry: &T) -> Result<Option<String>>
where
    T: BoundedTopologyValidation,
{
    ensure_topology_validation_budget(geometry)?;
    Ok(geometry
        .check_validation()
        .err()
        .map(|error| error.to_string()))
}

#[cfg(feature = "execute")]
pub(super) fn topology_is_valid<T>(geometry: &T) -> Result<bool>
where
    T: BoundedTopologyValidation,
{
    Ok(topology_validation_error(geometry)?.is_none())
}

#[cfg(feature = "execute")]
pub(super) fn topology_validation_errors(geometry: &Geometry<f64>) -> Result<Vec<String>> {
    ensure_topology_validation_budget(geometry)?;
    let mut errors = Vec::new();
    let truncated = geometry
        .visit_validation(Box::new(|error| {
            if errors.len() == MAX_TOPOLOGY_VALIDATION_ERRORS {
                return Err(());
            }
            errors.push(error.to_string());
            Ok(())
        }))
        .is_err();
    if truncated {
        errors.push(format!(
            "Additional topology validation errors were omitted after {MAX_TOPOLOGY_VALIDATION_ERRORS} results"
        ));
    }
    Ok(errors)
}

#[cfg(feature = "execute")]
pub(super) fn ensure_geometry_pair_budget(
    a: &Geometry<f64>,
    b: &Geometry<f64>,
    operation: &str,
) -> Result<()> {
    let a_coordinates = a.coords_count();
    let b_coordinates = b.coords_count();
    if a_coordinates != 0 && b_coordinates > MAX_TOPOLOGY_VALIDATION_WORK / a_coordinates {
        bail!(
            "{operation} exceeds the work limit of {MAX_TOPOLOGY_VALIDATION_WORK} potential coordinate-pair checks"
        );
    }
    Ok(())
}

#[cfg(all(test, feature = "execute"))]
mod validation_budget_tests {
    use super::*;
    use geo::{Coord, GeometryCollection, LineString};

    fn triangle() -> Polygon<f64> {
        Polygon::new(
            LineString::new(vec![
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 1.0, y: 0.0 },
                Coord { x: 0.0, y: 1.0 },
                Coord { x: 0.0, y: 0.0 },
            ]),
            Vec::new(),
        )
    }

    fn ring_with_segments(segments: usize) -> LineString<f64> {
        let mut coordinates = (0..segments)
            .map(|index| Coord {
                x: index as f64,
                y: (index % 2) as f64,
            })
            .collect::<Vec<_>>();
        coordinates.push(coordinates[0]);
        LineString::new(coordinates)
    }

    #[test]
    fn topology_budget_rejects_each_quadratic_geometry_shape() {
        let ring_within_limit =
            Geometry::Polygon(Polygon::new(ring_with_segments(1_414), Vec::new()));
        assert!(ensure_topology_validation_budget(&ring_within_limit).is_ok());
        let large_ring = Geometry::Polygon(Polygon::new(ring_with_segments(1_415), Vec::new()));
        assert!(ensure_topology_validation_budget(&large_ring).is_err());

        let hole_rich = Polygon::new(
            triangle().exterior().clone(),
            (0..666).map(|_| triangle().exterior().clone()).collect(),
        );
        assert!(ensure_topology_validation_budget(&hole_rich).is_err());

        let polygons_within_limit = MultiPolygon::new((0..666).map(|_| triangle()).collect());
        assert!(ensure_topology_validation_budget(&polygons_within_limit).is_ok());
        let many_polygons = MultiPolygon::new((0..667).map(|_| triangle()).collect());
        assert!(ensure_topology_validation_budget(&many_polygons).is_err());

        let nested = Geometry::GeometryCollection(GeometryCollection::new_from(vec![
            Geometry::Polygon(Polygon::new(ring_with_segments(1_001), Vec::new())),
            Geometry::GeometryCollection(GeometryCollection::new_from(vec![Geometry::Polygon(
                Polygon::new(ring_with_segments(1_001), Vec::new()),
            )])),
        ]));
        assert!(ensure_topology_validation_budget(&nested).is_err());
    }

    #[test]
    fn geometry_pair_budget_rejects_large_line_pairs() {
        let line = Geometry::LineString(ring_with_segments(1_415));
        assert!(ensure_geometry_pair_budget(&line, &line, "Test relation").is_err());
    }

    #[test]
    fn validation_errors_stop_at_the_diagnostic_limit() {
        let invalid_line = Geometry::LineString(LineString::new(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 0.0, y: 0.0 },
        ]));
        let collection = Geometry::GeometryCollection(GeometryCollection::new_from(vec![
            invalid_line;
            MAX_TOPOLOGY_VALIDATION_ERRORS + 10
        ]));
        let errors = topology_validation_errors(&collection).unwrap();
        assert_eq!(errors.len(), MAX_TOPOLOGY_VALIDATION_ERRORS + 1);
        assert!(errors.last().unwrap().contains("were omitted"));
    }
}
