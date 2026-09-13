//! Derived, line editing, geodesic, and planar transform operations for Geometry values.

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinType,
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    geometry::{GeometryKind, marker},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdvancedOperation {
    PointOnSurface,
    ClosestPoint,
    ConcaveHull,
    MinimumRotatedRectangle,
    Envelope,
    SimplifyVw,
    SimplifyVwPreserve,
    RemoveRepeatedPoints,
    ReverseLine,
    IsClosed,
    IsRing,
    InterpolateLine,
    LocatePoint,
    LineSubstring,
    DensifyPlanar,
    DensifyGeodesic,
    Bearing,
    Destination,
    GeodesicInterpolate,
    Translate,
    Rotate,
    Scale,
    Skew,
    AffineTransform,
    Triangulate,
    Smooth,
    SplitAntimeridian,
}

fn geometry_input(node: &mut Node, name: &str, kind: Option<GeometryKind>) {
    let pin = node.add_input_pin(
        name,
        name,
        "Validated WGS 84 longitude/latitude GeoJSON geometry",
        VariableType::Geometry,
    );
    pin.schema = kind.map(|kind| marker(kind).to_string());
}

fn geometry_output(node: &mut Node, kind: Option<GeometryKind>) {
    let pin = node.add_output_pin(
        "geometry_out",
        "Geometry",
        "Validated WGS 84 GeoJSON geometry",
        VariableType::Geometry,
    );
    pin.schema = kind.map(|kind| marker(kind).to_string());
}

fn number_input(node: &mut Node, name: &str) {
    node.add_input_pin(name, name, name, VariableType::Float);
}

fn definition(operation: AdvancedOperation) -> Node {
    use AdvancedOperation::*;
    let (id, alias, title, description) = match operation {
        PointOnSurface => (
            "geometry_point_on_surface",
            "pointOnSurface",
            "Point on Surface",
            "Returns a representative point that intersects the input and lies in its interior when possible. Empty geometry has no point on surface.",
        ),
        ClosestPoint => (
            "geometry_closest_point",
            "closestPoint",
            "Closest Point",
            "Returns the point on a geometry nearest to an input Point in the longitude/latitude coordinate plane.",
        ),
        ConcaveHull => (
            "geometry_concave_hull",
            "concaveHull",
            "Concave Hull",
            "Builds a planar concave hull around all input positions. Larger positive concavity values produce less detailed hulls. Degenerate inputs return their natural Point or LineString hull.",
        ),
        MinimumRotatedRectangle => (
            "geometry_minimum_rotated_rectangle",
            "minimumRotatedRectangle",
            "Minimum Rotated Bounding Geometry",
            "Returns the smallest-area rotated planar rectangle containing all positions. Degenerate inputs return a Point or LineString.",
        ),
        Envelope => (
            "geometry_envelope",
            "envelope",
            "Geometry Envelope",
            "Returns the axis-aligned planar bounds as a Polygon, LineString, or Point according to the input dimension. Empty input returns an empty GeometryCollection.",
        ),
        SimplifyVw => (
            "geometry_simplify_vw",
            "simplifyVw",
            "Simplify Geometry (Visvalingam-Whyatt)",
            "Simplifies line and polygon vertices using planar triangle areas in square coordinate degrees. The result is validated before it is returned.",
        ),
        SimplifyVwPreserve => (
            "geometry_simplify_vw_preserve",
            "simplifyVwPreserve",
            "Simplify Geometry (Topology-Aware VW)",
            "Uses the topology-aware Visvalingam-Whyatt variant, then validates the complete result. Epsilon is measured in square coordinate degrees.",
        ),
        RemoveRepeatedPoints => (
            "geometry_remove_repeated_points",
            "removeRepeatedPoints",
            "Remove Repeated Points",
            "Removes consecutive duplicate line and ring positions and duplicate MultiPoint members, then validates the result.",
        ),
        ReverseLine => (
            "geometry_reverse_line",
            "reverseLine",
            "Reverse LineString",
            "Returns a LineString with its positions in reverse order.",
        ),
        IsClosed => (
            "geometry_line_is_closed",
            "isClosed",
            "LineString Is Closed",
            "Tests whether the first and last LineString positions are equal.",
        ),
        IsRing => (
            "geometry_line_is_ring",
            "isRing",
            "LineString Is Ring",
            "Tests whether a LineString is closed and simple, with no crossings, self-touches, or overlapping segments.",
        ),
        InterpolateLine => (
            "geometry_interpolate_line",
            "interpolateLine",
            "Interpolate Point on LineString",
            "Returns a Point at a ratio from zero to one along a LineString using planar length.",
        ),
        LocatePoint => (
            "geometry_locate_point",
            "locatePoint",
            "Locate Point on LineString",
            "Returns the planar length ratio of the closest position on a LineString to an input Point.",
        ),
        LineSubstring => (
            "geometry_line_substring",
            "lineSubstring",
            "LineString Substring",
            "Extracts the part of a LineString between two planar length ratios. Ratios must satisfy 0 <= start < end <= 1.",
        ),
        DensifyPlanar => (
            "geometry_densify_planar",
            "densifyPlanar",
            "Densify LineString (Degrees)",
            "Adds positions so each planar LineString segment is no longer than the positive maximum length in coordinate degrees.",
        ),
        DensifyGeodesic => (
            "geometry_densify_geodesic",
            "densifyGeodesic",
            "Densify LineString (Meters)",
            "Adds points along WGS 84 geodesics so each segment is no longer than the positive maximum length in meters.",
        ),
        Bearing => (
            "geometry_geodesic_bearing",
            "geodesicBearing",
            "Geodesic Bearing",
            "Returns the initial WGS 84 geodesic bearing from one Point to another in degrees clockwise from north.",
        ),
        Destination => (
            "geometry_geodesic_destination",
            "geodesicDestination",
            "Geodesic Destination",
            "Returns the WGS 84 destination Point reached from an origin, initial bearing in degrees, and nonnegative distance in meters.",
        ),
        GeodesicInterpolate => (
            "geometry_geodesic_interpolate",
            "geodesicInterpolate",
            "Interpolate Geodesic Point",
            "Returns a Point at a ratio from zero to one along the WGS 84 geodesic between two Points.",
        ),
        Translate => (
            "geometry_translate",
            "translate",
            "Translate Geometry",
            "Offsets every position by planar longitude and latitude degrees. Results outside WGS 84 coordinate bounds fail validation.",
        ),
        Rotate => (
            "geometry_rotate",
            "rotate",
            "Rotate Geometry",
            "Rotates geometry counterclockwise in the coordinate plane around the center of its bounding box.",
        ),
        Scale => (
            "geometry_scale",
            "scale",
            "Scale Geometry",
            "Scales geometry in the coordinate plane around the center of its bounding box. Scale factors must be nonzero.",
        ),
        Skew => (
            "geometry_skew",
            "skew",
            "Skew Geometry",
            "Skews geometry by x and y angles in the coordinate plane around the center of its bounding box.",
        ),
        AffineTransform => (
            "geometry_affine_transform",
            "affineTransform",
            "Affine Transform Geometry",
            "Applies x' = a*x + b*y + x_offset and y' = d*x + e*y + y_offset. The matrix must be invertible.",
        ),
        Triangulate => (
            "geometry_triangulate",
            "triangulate",
            "Triangulate Polygon",
            "Triangulates Polygon or MultiPolygon input with an ear-cutting algorithm and returns a GeometryCollection of triangle Polygons.",
        ),
        Smooth => (
            "geometry_smooth_chaikin",
            "smoothChaikin",
            "Smooth Geometry (Chaikin)",
            "Applies Chaikin corner cutting to line and polygon geometry. Each iteration approximately doubles its position count.",
        ),
        SplitAntimeridian => (
            "geometry_split_line_antimeridian",
            "splitLineAntimeridian",
            "Split LineString at Antimeridian",
            "Splits LineString segments whose longitude jump exceeds 180 degrees and returns a MultiLineString with paired endpoints at longitude 180 and -180.",
        ),
    };

    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");

    match operation {
        PointOnSurface => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        ClosestPoint => {
            geometry_input(&mut node, "geometry", None);
            geometry_input(&mut node, "point", Some(GeometryKind::Point));
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        ConcaveHull => {
            geometry_input(&mut node, "geometry", None);
            number_input(&mut node, "concavity");
            geometry_output(&mut node, None);
        }
        MinimumRotatedRectangle | Envelope => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, None);
        }
        SimplifyVw | SimplifyVwPreserve => {
            geometry_input(&mut node, "geometry", None);
            number_input(&mut node, "epsilon");
            geometry_output(&mut node, None);
        }
        RemoveRepeatedPoints | Smooth => {
            geometry_input(&mut node, "geometry", None);
            if matches!(operation, Smooth) {
                node.add_input_pin(
                    "iterations",
                    "iterations",
                    "Number of smoothing iterations",
                    VariableType::Integer,
                );
            }
            geometry_output(&mut node, None);
        }
        ReverseLine | LineSubstring | DensifyPlanar | DensifyGeodesic | SplitAntimeridian => {
            geometry_input(&mut node, "geometry", Some(GeometryKind::LineString));
            match operation {
                LineSubstring => {
                    number_input(&mut node, "start_ratio");
                    number_input(&mut node, "end_ratio");
                }
                DensifyPlanar | DensifyGeodesic => number_input(&mut node, "max_length"),
                _ => {}
            }
            geometry_output(
                &mut node,
                Some(if matches!(operation, SplitAntimeridian) {
                    GeometryKind::MultiLineString
                } else {
                    GeometryKind::LineString
                }),
            );
        }
        IsClosed | IsRing => {
            geometry_input(&mut node, "geometry", Some(GeometryKind::LineString));
            node.add_output_pin(
                "result",
                "result",
                "Predicate result",
                VariableType::Boolean,
            );
        }
        InterpolateLine => {
            geometry_input(&mut node, "geometry", Some(GeometryKind::LineString));
            number_input(&mut node, "ratio");
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        LocatePoint => {
            geometry_input(&mut node, "geometry", Some(GeometryKind::LineString));
            geometry_input(&mut node, "point", Some(GeometryKind::Point));
            node.add_output_pin(
                "ratio",
                "ratio",
                "Planar length ratio from zero to one",
                VariableType::Float,
            );
        }
        Bearing | GeodesicInterpolate => {
            geometry_input(&mut node, "a", Some(GeometryKind::Point));
            geometry_input(&mut node, "b", Some(GeometryKind::Point));
            if matches!(operation, GeodesicInterpolate) {
                number_input(&mut node, "ratio");
                geometry_output(&mut node, Some(GeometryKind::Point));
            } else {
                node.add_output_pin(
                    "bearing",
                    "bearing",
                    "Initial bearing in degrees clockwise from north",
                    VariableType::Float,
                );
            }
        }
        Destination => {
            geometry_input(&mut node, "origin", Some(GeometryKind::Point));
            number_input(&mut node, "bearing");
            number_input(&mut node, "distance");
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        Translate => {
            geometry_input(&mut node, "geometry", None);
            number_input(&mut node, "longitude_offset");
            number_input(&mut node, "latitude_offset");
            geometry_output(&mut node, None);
        }
        Rotate => {
            geometry_input(&mut node, "geometry", None);
            number_input(&mut node, "degrees");
            geometry_output(&mut node, None);
        }
        Scale => {
            geometry_input(&mut node, "geometry", None);
            number_input(&mut node, "x_factor");
            number_input(&mut node, "y_factor");
            geometry_output(&mut node, None);
        }
        Skew => {
            geometry_input(&mut node, "geometry", None);
            number_input(&mut node, "x_degrees");
            number_input(&mut node, "y_degrees");
            geometry_output(&mut node, None);
        }
        AffineTransform => {
            geometry_input(&mut node, "geometry", None);
            for name in ["a", "b", "x_offset", "d", "e", "y_offset"] {
                number_input(&mut node, name);
            }
            geometry_output(&mut node, None);
        }
        Triangulate => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, Some(GeometryKind::GeometryCollection));
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
async fn run_operation(
    operation: AdvancedOperation,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    let mut values = flow_like_types::json::Map::new();
    for pin in definition(operation)
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input)
    {
        values.insert(
            pin.name.clone(),
            context
                .evaluate_pin::<flow_like_types::Value>(&pin.name)
                .await?,
        );
    }
    let outputs = super::cpu::run(move || {
        runtime::execute(operation, &flow_like_types::Value::Object(values))
    })
    .await?;
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
pub struct GeometryPointOnSurfaceNode;
implement_node!(
    GeometryPointOnSurfaceNode,
    AdvancedOperation::PointOnSurface
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryClosestPointNode;
implement_node!(GeometryClosestPointNode, AdvancedOperation::ClosestPoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryConcaveHullNode;
implement_node!(GeometryConcaveHullNode, AdvancedOperation::ConcaveHull);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryMinimumRotatedRectangleNode;
implement_node!(
    GeometryMinimumRotatedRectangleNode,
    AdvancedOperation::MinimumRotatedRectangle
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryEnvelopeNode;
implement_node!(GeometryEnvelopeNode, AdvancedOperation::Envelope);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySimplifyVwNode;
implement_node!(GeometrySimplifyVwNode, AdvancedOperation::SimplifyVw);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySimplifyVwPreserveNode;
implement_node!(
    GeometrySimplifyVwPreserveNode,
    AdvancedOperation::SimplifyVwPreserve
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryRemoveRepeatedPointsNode;
implement_node!(
    GeometryRemoveRepeatedPointsNode,
    AdvancedOperation::RemoveRepeatedPoints
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryReverseLineNode;
implement_node!(GeometryReverseLineNode, AdvancedOperation::ReverseLine);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryLineIsClosedNode;
implement_node!(GeometryLineIsClosedNode, AdvancedOperation::IsClosed);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryLineIsRingNode;
implement_node!(GeometryLineIsRingNode, AdvancedOperation::IsRing);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryInterpolateLineNode;
implement_node!(
    GeometryInterpolateLineNode,
    AdvancedOperation::InterpolateLine
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryLocatePointNode;
implement_node!(GeometryLocatePointNode, AdvancedOperation::LocatePoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryLineSubstringNode;
implement_node!(GeometryLineSubstringNode, AdvancedOperation::LineSubstring);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDensifyPlanarNode;
implement_node!(GeometryDensifyPlanarNode, AdvancedOperation::DensifyPlanar);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryDensifyGeodesicNode;
implement_node!(
    GeometryDensifyGeodesicNode,
    AdvancedOperation::DensifyGeodesic
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicBearingNode;
implement_node!(GeometryGeodesicBearingNode, AdvancedOperation::Bearing);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicDestinationNode;
implement_node!(
    GeometryGeodesicDestinationNode,
    AdvancedOperation::Destination
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicInterpolateNode;
implement_node!(
    GeometryGeodesicInterpolateNode,
    AdvancedOperation::GeodesicInterpolate
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryTranslateNode;
implement_node!(GeometryTranslateNode, AdvancedOperation::Translate);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryRotateNode;
implement_node!(GeometryRotateNode, AdvancedOperation::Rotate);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryScaleNode;
implement_node!(GeometryScaleNode, AdvancedOperation::Scale);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySkewNode;
implement_node!(GeometrySkewNode, AdvancedOperation::Skew);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryAffineTransformNode;
implement_node!(
    GeometryAffineTransformNode,
    AdvancedOperation::AffineTransform
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryTriangulateNode;
implement_node!(GeometryTriangulateNode, AdvancedOperation::Triangulate);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySmoothChaikinNode;
implement_node!(GeometrySmoothChaikinNode, AdvancedOperation::Smooth);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySplitLineAntimeridianNode;
implement_node!(
    GeometrySplitLineAntimeridianNode,
    AdvancedOperation::SplitAntimeridian
);

#[cfg(feature = "execute")]
pub(crate) use runtime::mixed_dimension_intersection;

#[cfg(feature = "execute")]
mod runtime {
    use super::super::{check_topology_validation, cpu};
    use super::AdvancedOperation;
    use flow_like_geometry::{from_geo, to_geo};
    use flow_like_types::{
        Result, Value, anyhow, bail,
        geometry::{MAX_GEOMETRY_POSITIONS, canonicalize_geometry},
        json::json,
    };
    use geo::{
        AffineOps, AffineTransform as GeoAffineTransform, Bearing, BooleanOps, BoundingRect,
        ChaikinSmoothing, Closest, ClosestPoint, ConcaveHull, Coord, CoordsIter, Destination,
        Distance, Euclidean, Geodesic, Geometry, GeometryCollection, InteriorPoint,
        InterpolateLine, InterpolatePoint, Intersects, Kernel, Length, Line, LineString,
        MinimumRotatedRect, MultiLineString, MultiPoint, MultiPolygon, Orientation, Point, Polygon,
        Relate, RemoveRepeatedPoints, Rotate, Scale, SimplifyVw, SimplifyVwPreserve, Skew,
        Translate, TriangulateEarcut,
        algorithm::{kernels::RobustKernel, unary_union},
        line_intersection::{LineIntersection, line_intersection},
    };
    use std::{
        collections::HashSet,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    const MAX_INTERSECTION_COMPARISONS: usize = 2_000_000;
    const INTERSECTION_BATCH_SIZE: usize = 256;

    macro_rules! ensure {
        ($condition:expr, $($message:tt)*) => {
            if !$condition { bail!($($message)*); }
        };
    }

    fn input<'a>(inputs: &'a Value, name: &str) -> Result<&'a Value> {
        inputs
            .get(name)
            .ok_or_else(|| anyhow!("Missing geometry input {name}"))
    }

    fn number(inputs: &Value, name: &str) -> Result<f64> {
        let value = input(inputs, name)?
            .as_f64()
            .ok_or_else(|| anyhow!("{name} must be a finite number"))?;
        ensure!(value.is_finite(), "{name} must be finite");
        Ok(value)
    }

    fn nonnegative_integer(inputs: &Value, name: &str) -> Result<usize> {
        let value = input(inputs, name)?
            .as_i64()
            .ok_or_else(|| anyhow!("{name} must be a nonnegative integer"))?;
        ensure!(value >= 0, "{name} must be nonnegative");
        usize::try_from(value).map_err(Into::into)
    }

    fn checked_geometry(value: &Value) -> Result<Geometry<f64>> {
        let geometry = to_geo(&canonicalize_geometry(value, None)?)?;
        check_topology_validation(&geometry, "Invalid geometry for spatial operation")?;
        Ok(geometry)
    }

    fn checked_line(value: &Value) -> Result<LineString<f64>> {
        match checked_geometry(value)? {
            Geometry::LineString(line) => Ok(line),
            _ => bail!("This operation requires a LineString"),
        }
    }

    fn checked_point(value: &Value) -> Result<Point<f64>> {
        match checked_geometry(value)? {
            Geometry::Point(point) => Ok(point),
            _ => bail!("This operation requires a Point"),
        }
    }

    fn encoded_geometry(geometry: Geometry<f64>) -> Result<Value> {
        check_topology_validation(&geometry, "Spatial operation produced invalid geometry")?;
        canonicalize_geometry(&from_geo(&geometry)?, None).map_err(Into::into)
    }

    fn geometry_result(geometry: Geometry<f64>) -> Result<Vec<(&'static str, Value)>> {
        Ok(vec![("geometry_out", encoded_geometry(geometry)?)])
    }

    fn unique_coords(geometry: &Geometry<f64>) -> Vec<Coord<f64>> {
        let mut coords: Vec<_> = geometry.coords_iter().collect();
        coords.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
        coords.dedup();
        coords
    }

    fn linear_hull(coords: &[Coord<f64>]) -> Option<Geometry<f64>> {
        match coords {
            [] => Some(Geometry::GeometryCollection(GeometryCollection::empty())),
            [coord] => Some(Geometry::Point((*coord).into())),
            [first, rest @ ..] => {
                let second = rest[0];
                if rest.iter().all(|coord| {
                    RobustKernel::orient2d(*first, second, *coord) == Orientation::Collinear
                }) {
                    Some(Geometry::LineString(LineString::new(vec![
                        *first,
                        *coords.last().expect("nonempty coordinate slice"),
                    ])))
                } else {
                    None
                }
            }
        }
    }

    fn concave_hull(geometry: &Geometry<f64>, concavity: f64) -> Geometry<f64> {
        let coords = unique_coords(geometry);
        if let Some(linear) = linear_hull(&coords) {
            return linear;
        }
        let points = MultiPoint::new(coords.into_iter().map(Point::from).collect());
        Geometry::Polygon(points.concave_hull(concavity))
    }

    fn minimum_rotated_geometry(geometry: &Geometry<f64>) -> Result<Geometry<f64>> {
        let coords = unique_coords(geometry);
        if let Some(linear) = linear_hull(&coords) {
            return Ok(linear);
        }
        geometry
            .minimum_rotated_rect()
            .map(Geometry::Polygon)
            .ok_or_else(|| anyhow!("Geometry has no minimum rotated rectangle"))
    }

    fn envelope(geometry: &Geometry<f64>) -> Geometry<f64> {
        let Some(bounds) = geometry.bounding_rect() else {
            return Geometry::GeometryCollection(GeometryCollection::empty());
        };
        let min = bounds.min();
        let max = bounds.max();
        if min == max {
            Geometry::Point(min.into())
        } else if min.x == max.x || min.y == max.y {
            Geometry::LineString(LineString::new(vec![min, max]))
        } else {
            Geometry::Polygon(bounds.to_polygon())
        }
    }

    fn simplify_vw_geometry(
        geometry: Geometry<f64>,
        epsilon: f64,
        preserve: bool,
    ) -> Result<Geometry<f64>> {
        let result = match geometry {
            Geometry::LineString(value) => {
                if preserve {
                    value.simplify_vw_preserve(epsilon).into()
                } else {
                    value.simplify_vw(epsilon).into()
                }
            }
            Geometry::MultiLineString(value) => {
                if preserve {
                    value.simplify_vw_preserve(epsilon).into()
                } else {
                    value.simplify_vw(epsilon).into()
                }
            }
            Geometry::Polygon(value) => {
                if preserve {
                    value.simplify_vw_preserve(epsilon).into()
                } else {
                    value.simplify_vw(epsilon).into()
                }
            }
            Geometry::MultiPolygon(value) => {
                if preserve {
                    value.simplify_vw_preserve(epsilon).into()
                } else {
                    value.simplify_vw(epsilon).into()
                }
            }
            Geometry::GeometryCollection(collection) => {
                Geometry::GeometryCollection(GeometryCollection::new_from(
                    collection
                        .0
                        .into_iter()
                        .map(|member| simplify_vw_geometry(member, epsilon, preserve))
                        .collect::<Result<Vec<_>>>()?,
                ))
            }
            other => other,
        };
        Ok(result)
    }

    fn line_is_ring(line: &LineString<f64>) -> Result<bool> {
        if line.0.len() < 4 || line.0.first() != line.0.last() {
            return Ok(false);
        }
        let segments: Vec<_> = line.lines().collect();
        let comparisons = segments
            .len()
            .saturating_mul(segments.len().saturating_sub(1))
            / 2;
        ensure!(
            comparisons <= MAX_INTERSECTION_COMPARISONS,
            "Ring validation exceeds the segment comparison limit"
        );
        let has_invalid_intersection = cpu::any_indexed(segments.len(), comparisons, |i| {
            let first = segments[i];
            for (j, second) in segments.iter().enumerate().skip(i + 1) {
                let adjacent = j == i + 1 || (i == 0 && j + 1 == segments.len());
                let Some(intersection) = line_intersection(first, *second) else {
                    continue;
                };
                if !adjacent {
                    return true;
                }
                match intersection {
                    LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    } => {
                        let expected = if i == 0 && j + 1 == segments.len() {
                            first.start
                        } else {
                            first.end
                        };
                        if is_proper || intersection != expected {
                            return true;
                        }
                    }
                    LineIntersection::Collinear { .. } => return true,
                }
            }
            false
        });
        Ok(!has_invalid_intersection)
    }

    fn locate_point(line: &LineString<f64>, point: Point<f64>) -> f64 {
        let total = Euclidean.length(line);
        let mut traversed = 0.0;
        let mut best_distance = f64::INFINITY;
        let mut best_along = 0.0;
        for segment in line.lines() {
            let dx = segment.end.x - segment.start.x;
            let dy = segment.end.y - segment.start.y;
            let squared = dx * dx + dy * dy;
            let ratio = if squared == 0.0 {
                0.0
            } else {
                (((point.x() - segment.start.x) * dx + (point.y() - segment.start.y) * dy)
                    / squared)
                    .clamp(0.0, 1.0)
            };
            let closest = Point::new(segment.start.x + ratio * dx, segment.start.y + ratio * dy);
            let distance = Euclidean.distance(point, closest);
            let segment_length = squared.sqrt();
            if distance < best_distance {
                best_distance = distance;
                best_along = traversed + ratio * segment_length;
            }
            traversed += segment_length;
        }
        best_along / total
    }

    fn point_at_line_ratio(
        line: &LineString<f64>,
        ratio: f64,
        missing_message: &str,
    ) -> Result<Point<f64>> {
        if ratio == 0.0 {
            return Ok(Point(line.0[0]));
        }
        if ratio == 1.0 {
            return Ok(Point(*line.0.last().expect("validated LineString")));
        }
        Euclidean
            .point_at_ratio_from_start(line, ratio)
            .ok_or_else(|| anyhow!("{missing_message}"))
    }

    fn line_substring(line: &LineString<f64>, start: f64, end: f64) -> Result<LineString<f64>> {
        let total = Euclidean.length(line);
        ensure!(total > 0.0, "A zero-length LineString has no substring");
        let start_point =
            point_at_line_ratio(line, start, "LineString has no interpolated start point")?;
        let end_point = point_at_line_ratio(line, end, "LineString has no interpolated end point")?;
        let mut result = vec![start_point.0];
        let mut traversed = 0.0;
        for segment in line.lines() {
            traversed += Euclidean.length(&segment);
            let ratio = traversed / total;
            if ratio > start && ratio < end && result.last() != Some(&segment.end) {
                result.push(segment.end);
            }
        }
        if result.last() != Some(&end_point.0) {
            result.push(end_point.0);
        }
        ensure!(
            result.len() >= 2 && result.iter().skip(1).any(|coord| coord != &result[0]),
            "Substring collapsed to a single position"
        );
        Ok(LineString::new(result))
    }

    fn densify_line(
        line: &LineString<f64>,
        max_length: f64,
        geodesic: bool,
    ) -> Result<LineString<f64>> {
        let mut output = Vec::with_capacity(line.0.len());
        output.push(line.0[0]);
        for pair in line.0.windows(2) {
            let start = Point::from(pair[0]);
            let end = Point::from(pair[1]);
            let distance = if geodesic {
                Geodesic.distance(start, end)
            } else {
                Euclidean.distance(start, end)
            };
            let pieces = (distance / max_length).ceil().max(1.0);
            ensure!(
                pieces.is_finite() && pieces <= MAX_GEOMETRY_POSITIONS as f64,
                "Densification would exceed the Geometry position limit"
            );
            let pieces = pieces as usize;
            ensure!(
                output.len().saturating_add(pieces) <= MAX_GEOMETRY_POSITIONS,
                "Densification would exceed the Geometry position limit"
            );
            for index in 1..pieces {
                let ratio = index as f64 / pieces as f64;
                let point = if geodesic {
                    Geodesic.point_at_ratio_between(start, end, ratio)
                } else {
                    Euclidean.point_at_ratio_between(start, end, ratio)
                };
                output.push(point.0);
            }
            output.push(pair[1]);
        }
        Ok(LineString::new(output))
    }

    fn smoothed_line_position_count(line: &LineString<f64>, iterations: usize) -> usize {
        if line.0.is_empty() || iterations == 0 {
            return line.0.len();
        }
        let closed = line.0.first() == line.0.last();
        (0..iterations).fold(line.0.len(), |count, _| {
            if closed {
                count.saturating_sub(1).saturating_mul(2).saturating_add(1)
            } else {
                count.saturating_mul(2)
            }
        })
    }

    fn smoothed_position_count(geometry: &Geometry<f64>, iterations: usize) -> usize {
        match geometry {
            Geometry::LineString(line) => smoothed_line_position_count(line, iterations),
            Geometry::MultiLineString(lines) => lines.0.iter().fold(0usize, |count, line| {
                count.saturating_add(smoothed_line_position_count(line, iterations))
            }),
            Geometry::Polygon(polygon) => std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .fold(0usize, |count, ring| {
                    count.saturating_add(smoothed_line_position_count(ring, iterations))
                }),
            Geometry::MultiPolygon(polygons) => polygons.0.iter().fold(0usize, |count, polygon| {
                count.saturating_add(
                    std::iter::once(polygon.exterior())
                        .chain(polygon.interiors())
                        .fold(0usize, |count, ring| {
                            count.saturating_add(smoothed_line_position_count(ring, iterations))
                        }),
                )
            }),
            Geometry::GeometryCollection(collection) => {
                collection.0.iter().fold(0usize, |count, member| {
                    count.saturating_add(smoothed_position_count(member, iterations))
                })
            }
            other => other.coords_iter().count(),
        }
    }

    fn smooth_geometry(geometry: Geometry<f64>, iterations: usize) -> Geometry<f64> {
        match geometry {
            Geometry::GeometryCollection(collection) => {
                Geometry::GeometryCollection(GeometryCollection::new_from(
                    collection
                        .0
                        .into_iter()
                        .map(|member| smooth_geometry(member, iterations))
                        .collect(),
                ))
            }
            other => other.chaikin_smoothing(iterations),
        }
    }

    fn triangulate(geometry: Geometry<f64>) -> Result<Geometry<f64>> {
        let polygons: Vec<Polygon<f64>> = match geometry {
            Geometry::Polygon(polygon) => vec![polygon],
            Geometry::MultiPolygon(polygons) => polygons.0,
            _ => bail!("Triangulate requires Polygon or MultiPolygon"),
        };
        // Every triangle contributes four positions including its closing vertex.
        let triangle_bound = polygons.iter().fold(0usize, |count, polygon| {
            let vertices = std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .fold(0usize, |vertices, ring| {
                    vertices.saturating_add(ring.0.len().saturating_sub(1))
                });
            let holes = polygon
                .interiors()
                .iter()
                .filter(|ring| !ring.0.is_empty())
                .count();
            count.saturating_add(
                vertices
                    .saturating_add(holes.saturating_mul(2))
                    .saturating_sub(2),
            )
        });
        ensure!(
            triangle_bound <= MAX_GEOMETRY_POSITIONS / 4,
            "Triangulation would exceed the Geometry position limit"
        );
        let work = polygons.iter().fold(0usize, |work, polygon| {
            let vertices = polygon.coords_count();
            work.saturating_add(vertices.saturating_mul(vertices))
        });
        let triangles = cpu::map_ordered(&polygons, work, |_, polygon| {
            polygon
                .earcut_triangles()
                .into_iter()
                .map(|triangle| Geometry::Polygon(triangle.to_polygon()))
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect();
        Ok(Geometry::GeometryCollection(GeometryCollection::new_from(
            triangles,
        )))
    }

    fn has_line_extent(coordinates: &[Coord<f64>]) -> bool {
        coordinates
            .first()
            .is_some_and(|first| coordinates.iter().skip(1).any(|coord| coord != first))
    }

    fn split_antimeridian(line: LineString<f64>) -> Result<Geometry<f64>> {
        let mut parts: Vec<LineString<f64>> = Vec::new();
        let mut current = vec![line.0[0]];
        for pair in line.0.windows(2) {
            let mut start = pair[0];
            if let Some(previous) = current.last().copied()
                && previous.y == start.y
                && (previous.x - start.x).abs() == 360.0
            {
                start = previous;
            }
            let end = pair[1];
            let delta = end.x - start.x;
            if delta.abs() <= 180.0 {
                if current.last() != Some(&end) {
                    current.push(end);
                }
                continue;
            }

            let (unwrapped_end, boundary, opposite) = if delta < -180.0 {
                (end.x + 360.0, 180.0, -180.0)
            } else {
                (end.x - 360.0, -180.0, 180.0)
            };
            let denominator = unwrapped_end - start.x;
            if denominator == 0.0 {
                let rebased_end = Coord {
                    x: start.x,
                    y: end.y,
                };
                if current.last() != Some(&rebased_end) {
                    current.push(rebased_end);
                }
                continue;
            }
            let ratio = (boundary - start.x) / denominator;
            ensure!(
                (0.0..=1.0).contains(&ratio),
                "Antimeridian crossing could not be resolved"
            );
            let latitude = if ratio == 0.0 {
                start.y
            } else if ratio == 1.0 {
                end.y
            } else {
                start.y + ratio * (end.y - start.y)
            };
            let first_boundary = Coord {
                x: boundary,
                y: latitude,
            };
            let second_boundary = Coord {
                x: opposite,
                y: latitude,
            };
            if ratio == 0.0 {
                if has_line_extent(&current) {
                    parts.push(LineString::new(std::mem::take(&mut current)));
                }
                current = vec![second_boundary];
                if current.last() != Some(&end) {
                    current.push(end);
                }
                continue;
            }
            if current.last() != Some(&first_boundary) {
                current.push(first_boundary);
            }
            ensure!(
                has_line_extent(&current),
                "Antimeridian split produced a collapsed line part"
            );
            parts.push(LineString::new(std::mem::take(&mut current)));
            current = vec![second_boundary];
            if ratio != 1.0 && current.last() != Some(&end) {
                current.push(end);
            }
        }
        if has_line_extent(&current) {
            parts.push(LineString::new(current));
        }
        ensure!(
            !parts.is_empty(),
            "Antimeridian split produced no nonzero line parts"
        );
        Ok(Geometry::MultiLineString(geo::MultiLineString::new(parts)))
    }

    pub(super) fn execute(
        operation: AdvancedOperation,
        inputs: &Value,
    ) -> Result<Vec<(&'static str, Value)>> {
        use AdvancedOperation::*;
        match operation {
            PointOnSurface => {
                let point = checked_geometry(input(inputs, "geometry")?)?
                    .interior_point()
                    .ok_or_else(|| anyhow!("Empty geometry has no point on surface"))?;
                geometry_result(point.into())
            }
            ClosestPoint => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                let point = checked_point(input(inputs, "point")?)?;
                let closest = match geometry.closest_point(&point) {
                    Closest::Intersection(point) | Closest::SinglePoint(point) => point,
                    Closest::Indeterminate => {
                        bail!("Empty or degenerate geometry has no unique closest point")
                    }
                };
                geometry_result(closest.into())
            }
            ConcaveHull => {
                let concavity = number(inputs, "concavity")?;
                ensure!(concavity > 0.0, "Concavity must be greater than zero");
                geometry_result(concave_hull(
                    &checked_geometry(input(inputs, "geometry")?)?,
                    concavity,
                ))
            }
            MinimumRotatedRectangle => geometry_result(minimum_rotated_geometry(
                &checked_geometry(input(inputs, "geometry")?)?,
            )?),
            Envelope => geometry_result(envelope(&checked_geometry(input(inputs, "geometry")?)?)),
            SimplifyVw | SimplifyVwPreserve => {
                let epsilon = number(inputs, "epsilon")?;
                ensure!(epsilon >= 0.0, "Simplification epsilon must be nonnegative");
                let geometry = simplify_vw_geometry(
                    checked_geometry(input(inputs, "geometry")?)?,
                    epsilon,
                    matches!(operation, SimplifyVwPreserve),
                )?;
                geometry_result(geometry)
            }
            RemoveRepeatedPoints => geometry_result(
                checked_geometry(input(inputs, "geometry")?)?.remove_repeated_points(),
            ),
            ReverseLine => {
                let mut line = checked_line(input(inputs, "geometry")?)?;
                line.0.reverse();
                geometry_result(line.into())
            }
            IsClosed => {
                let line = checked_line(input(inputs, "geometry")?)?;
                Ok(vec![("result", json!(line.0.first() == line.0.last()))])
            }
            IsRing => {
                let line = checked_line(input(inputs, "geometry")?)?;
                Ok(vec![("result", json!(line_is_ring(&line)?))])
            }
            InterpolateLine => {
                let ratio = number(inputs, "ratio")?;
                ensure!(
                    (0.0..=1.0).contains(&ratio),
                    "Ratio must be between zero and one"
                );
                let line = checked_line(input(inputs, "geometry")?)?;
                let point =
                    point_at_line_ratio(&line, ratio, "LineString has no interpolated point")?;
                geometry_result(point.into())
            }
            LocatePoint => {
                let line = checked_line(input(inputs, "geometry")?)?;
                let point = checked_point(input(inputs, "point")?)?;
                Ok(vec![("ratio", json!(locate_point(&line, point)))])
            }
            LineSubstring => {
                let start = number(inputs, "start_ratio")?;
                let end = number(inputs, "end_ratio")?;
                ensure!(
                    start >= 0.0 && start < end && end <= 1.0,
                    "Ratios must satisfy 0 <= start_ratio < end_ratio <= 1"
                );
                geometry_result(
                    line_substring(&checked_line(input(inputs, "geometry")?)?, start, end)?.into(),
                )
            }
            DensifyPlanar | DensifyGeodesic => {
                let max_length = number(inputs, "max_length")?;
                ensure!(
                    max_length > 0.0,
                    "Maximum segment length must be greater than zero"
                );
                geometry_result(
                    densify_line(
                        &checked_line(input(inputs, "geometry")?)?,
                        max_length,
                        matches!(operation, DensifyGeodesic),
                    )?
                    .into(),
                )
            }
            Bearing => {
                let a = checked_point(input(inputs, "a")?)?;
                let b = checked_point(input(inputs, "b")?)?;
                ensure!(a != b, "Bearing is undefined for coincident Points");
                Ok(vec![(
                    "bearing",
                    json!(Geodesic.bearing(a, b).rem_euclid(360.0)),
                )])
            }
            Destination => {
                let origin = checked_point(input(inputs, "origin")?)?;
                let bearing = number(inputs, "bearing")?;
                let distance = number(inputs, "distance")?;
                ensure!(distance >= 0.0, "Distance must be nonnegative meters");
                geometry_result(Geodesic.destination(origin, bearing, distance).into())
            }
            GeodesicInterpolate => {
                let a = checked_point(input(inputs, "a")?)?;
                let b = checked_point(input(inputs, "b")?)?;
                let ratio = number(inputs, "ratio")?;
                ensure!(
                    (0.0..=1.0).contains(&ratio),
                    "Ratio must be between zero and one"
                );
                geometry_result(Geodesic.point_at_ratio_between(a, b, ratio).into())
            }
            Translate => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                geometry_result(geometry.translate(
                    number(inputs, "longitude_offset")?,
                    number(inputs, "latitude_offset")?,
                ))
            }
            Rotate => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                geometry_result(geometry.rotate_around_center(number(inputs, "degrees")?))
            }
            Scale => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                let x = number(inputs, "x_factor")?;
                let y = number(inputs, "y_factor")?;
                ensure!(x != 0.0 && y != 0.0, "Scale factors must be nonzero");
                geometry_result(geometry.scale_xy(x, y))
            }
            Skew => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                let x = number(inputs, "x_degrees")?;
                let y = number(inputs, "y_degrees")?;
                ensure!(
                    x.to_radians().cos().abs() > 1e-12 && y.to_radians().cos().abs() > 1e-12,
                    "Skew angles cannot be an odd multiple of 90 degrees"
                );
                geometry_result(geometry.skew_xy(x, y))
            }
            AffineTransform => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                let a = number(inputs, "a")?;
                let b = number(inputs, "b")?;
                let d = number(inputs, "d")?;
                let e = number(inputs, "e")?;
                ensure!(
                    a * e - b * d != 0.0,
                    "Affine transform matrix must be invertible"
                );
                let transform = GeoAffineTransform::new(
                    a,
                    b,
                    number(inputs, "x_offset")?,
                    d,
                    e,
                    number(inputs, "y_offset")?,
                );
                geometry_result(geometry.affine_transform(&transform))
            }
            Triangulate => {
                geometry_result(triangulate(checked_geometry(input(inputs, "geometry")?)?)?)
            }
            Smooth => {
                let geometry = checked_geometry(input(inputs, "geometry")?)?;
                let iterations = nonnegative_integer(inputs, "iterations")?;
                ensure!(iterations <= 16, "Smoothing supports at most 16 iterations");
                ensure!(
                    smoothed_position_count(&geometry, iterations) <= MAX_GEOMETRY_POSITIONS,
                    "Smoothing would exceed the Geometry position limit"
                );
                geometry_result(smooth_geometry(geometry, iterations))
            }
            SplitAntimeridian => split_antimeridian(checked_line(input(inputs, "geometry")?)?)
                .and_then(geometry_result),
        }
    }

    #[derive(Default)]
    struct IntersectionCounters {
        comparisons: AtomicUsize,
        emitted_positions: AtomicUsize,
    }

    #[derive(Clone, Default)]
    struct IntersectionBudget {
        counters: Arc<IntersectionCounters>,
    }

    impl IntersectionBudget {
        fn reserve(counter: &AtomicUsize, count: usize, limit: usize) -> Result<()> {
            counter
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    used.checked_add(count).filter(|next| *next <= limit)
                })
                .map(|_| ())
                .map_err(|_| anyhow!(
                    "Mixed-dimensional intersection exceeds its segment comparison or Geometry position limit"
                ))
        }

        fn comparisons(&mut self, count: usize) -> Result<()> {
            Self::reserve(
                &self.counters.comparisons,
                count,
                MAX_INTERSECTION_COMPARISONS,
            )
        }

        fn emit_positions(&mut self, count: usize) -> Result<()> {
            Self::reserve(
                &self.counters.emitted_positions,
                count,
                MAX_GEOMETRY_POSITIONS,
            )
        }
    }

    fn atomic_geometries(geometry: &Geometry<f64>, output: &mut Vec<Geometry<f64>>) {
        match geometry {
            Geometry::MultiPoint(points) => {
                output.extend(points.0.iter().copied().map(Geometry::Point));
            }
            Geometry::MultiLineString(lines) => {
                output.extend(lines.0.iter().cloned().map(Geometry::LineString));
            }
            Geometry::MultiPolygon(polygons) => {
                output.extend(polygons.0.iter().cloned().map(Geometry::Polygon));
            }
            Geometry::GeometryCollection(collection) => {
                for member in &collection.0 {
                    atomic_geometries(member, output);
                }
            }
            Geometry::Point(_) | Geometry::LineString(_) | Geometry::Polygon(_) => {
                output.push(geometry.clone());
            }
            _ => {}
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct CoordKey(u64, u64);

    fn coord_key(coord: Coord<f64>) -> CoordKey {
        let bits = |value: f64| if value == 0.0 { 0.0 } else { value }.to_bits();
        CoordKey(bits(coord.x), bits(coord.y))
    }

    fn canonical_line_key(line: &LineString<f64>) -> Vec<CoordKey> {
        let forward: Vec<_> = line.0.iter().copied().map(coord_key).collect();
        let reverse: Vec<_> = forward.iter().copied().rev().collect();
        forward.min(reverse)
    }

    fn canonical_ring_key(ring: &LineString<f64>) -> Vec<CoordKey> {
        let mut coordinates: Vec<_> = ring.0.iter().copied().map(coord_key).collect();
        if coordinates.len() > 1 && coordinates.first() == coordinates.last() {
            coordinates.pop();
        }
        if coordinates.is_empty() {
            return coordinates;
        }
        let rotate_from_minimum = |coordinates: &[CoordKey]| {
            let start = coordinates
                .iter()
                .enumerate()
                .min_by_key(|(_, coordinate)| **coordinate)
                .map(|(index, _)| index)
                .unwrap_or(0);
            coordinates[start..]
                .iter()
                .chain(&coordinates[..start])
                .copied()
                .collect::<Vec<_>>()
        };
        let forward = rotate_from_minimum(&coordinates);
        coordinates.reverse();
        forward.min(rotate_from_minimum(&coordinates))
    }

    #[derive(Debug, PartialEq, Eq, Hash)]
    enum IntersectionGeometryKey {
        Point(CoordKey),
        LineString(Vec<CoordKey>),
        Polygon(Vec<Vec<CoordKey>>),
    }

    fn intersection_geometry_key(geometry: &Geometry<f64>) -> Option<IntersectionGeometryKey> {
        match geometry {
            Geometry::Point(point) => Some(IntersectionGeometryKey::Point(coord_key(point.0))),
            Geometry::LineString(line) => Some(IntersectionGeometryKey::LineString(
                canonical_line_key(line),
            )),
            Geometry::Polygon(polygon) => {
                let exterior = canonical_ring_key(polygon.exterior());
                let mut interiors: Vec<_> =
                    polygon.interiors().iter().map(canonical_ring_key).collect();
                interiors.sort();
                let mut rings = Vec::with_capacity(interiors.len() + 1);
                rings.push(exterior);
                rings.extend(interiors);
                Some(IntersectionGeometryKey::Polygon(rings))
            }
            _ => None,
        }
    }

    fn push_unique_point(
        points: &mut Vec<Point<f64>>,
        seen: &mut HashSet<CoordKey>,
        point: Point<f64>,
        budget: &mut IntersectionBudget,
    ) -> Result<()> {
        if seen.insert(coord_key(point.0)) {
            budget.emit_positions(1)?;
            points.push(point);
        }
        Ok(())
    }

    fn push_unique_line(
        lines: &mut Vec<LineString<f64>>,
        seen: &mut HashSet<Vec<CoordKey>>,
        line: LineString<f64>,
        budget: &mut IntersectionBudget,
    ) -> Result<()> {
        if seen.insert(canonical_line_key(&line)) {
            budget.emit_positions(line.0.len())?;
            lines.push(line);
        }
        Ok(())
    }

    fn record_line_intersection(
        first: Line<f64>,
        second: Line<f64>,
        points: &mut Vec<Point<f64>>,
        lines: &mut Vec<LineString<f64>>,
        point_keys: &mut HashSet<CoordKey>,
        line_keys: &mut HashSet<Vec<CoordKey>>,
        budget: &mut IntersectionBudget,
    ) -> Result<()> {
        match line_intersection(first, second) {
            Some(LineIntersection::SinglePoint { intersection, .. }) => {
                push_unique_point(points, point_keys, intersection.into(), budget)?;
            }
            Some(LineIntersection::Collinear { intersection }) => {
                if intersection.start != intersection.end {
                    push_unique_line(
                        lines,
                        line_keys,
                        LineString::new(vec![intersection.start, intersection.end]),
                        budget,
                    )?;
                }
            }
            None => {}
        }
        Ok(())
    }

    fn intersection_parameter(line: Line<f64>, coord: Coord<f64>) -> f64 {
        let dx = line.end.x - line.start.x;
        let dy = line.end.y - line.start.y;
        if dx.abs() >= dy.abs() && dx != 0.0 {
            (coord.x - line.start.x) / dx
        } else if dy != 0.0 {
            (coord.y - line.start.y) / dy
        } else {
            0.0
        }
    }

    fn line_line_parts(
        a: &LineString<f64>,
        b: &LineString<f64>,
        budget: &mut IntersectionBudget,
    ) -> Result<Vec<Geometry<f64>>> {
        budget.comparisons(
            a.0.len()
                .saturating_sub(1)
                .saturating_mul(b.0.len().saturating_sub(1)),
        )?;
        let mut points = Vec::new();
        let mut lines = Vec::new();
        let mut point_keys = HashSet::new();
        let mut line_keys = HashSet::new();
        for first in a.lines() {
            for second in b.lines() {
                record_line_intersection(
                    first,
                    second,
                    &mut points,
                    &mut lines,
                    &mut point_keys,
                    &mut line_keys,
                    budget,
                )?;
            }
        }
        let mut uncovered_points = Vec::with_capacity(points.len());
        'point: for point in points {
            for line in &lines {
                budget.comparisons(line.coords_count().max(1))?;
                if line.intersects(&point) {
                    continue 'point;
                }
            }
            uncovered_points.push(point);
        }
        Ok(lines
            .into_iter()
            .map(Geometry::LineString)
            .chain(uncovered_points.into_iter().map(Geometry::Point))
            .collect())
    }

    fn polygon_boundary(polygon: &Polygon<f64>) -> impl Iterator<Item = Line<f64>> + '_ {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .flat_map(|ring| ring.lines())
    }

    fn line_polygon_parts(
        line: &LineString<f64>,
        polygon: &Polygon<f64>,
        budget: &mut IntersectionBudget,
    ) -> Result<Vec<Geometry<f64>>> {
        let boundary: Vec<_> = polygon_boundary(polygon).collect();
        budget.comparisons(
            line.0
                .len()
                .saturating_sub(1)
                .saturating_mul(boundary.len()),
        )?;
        let mut output_lines: Vec<LineString<f64>> = Vec::new();
        let mut contact_points: Vec<Point<f64>> = Vec::new();
        let mut contact_point_keys = HashSet::new();

        for segment in line.lines() {
            let mut ratios = vec![0.0, 1.0];
            for edge in &boundary {
                match line_intersection(segment, *edge) {
                    Some(LineIntersection::SinglePoint { intersection, .. }) => {
                        ratios.push(intersection_parameter(segment, intersection).clamp(0.0, 1.0));
                        push_unique_point(
                            &mut contact_points,
                            &mut contact_point_keys,
                            intersection.into(),
                            budget,
                        )?;
                    }
                    Some(LineIntersection::Collinear { intersection }) => {
                        for coord in [intersection.start, intersection.end] {
                            ratios.push(intersection_parameter(segment, coord).clamp(0.0, 1.0));
                            push_unique_point(
                                &mut contact_points,
                                &mut contact_point_keys,
                                coord.into(),
                                budget,
                            )?;
                        }
                    }
                    None => {}
                }
            }
            ratios.sort_by(f64::total_cmp);
            ratios.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
            for pair in ratios.windows(2) {
                if pair[1] - pair[0] <= 1e-12 {
                    continue;
                }
                let at = |ratio: f64| Coord {
                    x: segment.start.x + ratio * (segment.end.x - segment.start.x),
                    y: segment.start.y + ratio * (segment.end.y - segment.start.y),
                };
                let start = at(pair[0]);
                let end = at(pair[1]);
                let midpoint = Point::from(at((pair[0] + pair[1]) / 2.0));
                budget.comparisons(boundary.len())?;
                if polygon.intersects(&midpoint) {
                    if let Some(previous) = output_lines.last_mut()
                        && previous.0.last() == Some(&start)
                    {
                        if previous.0.last() != Some(&end) {
                            budget.emit_positions(1)?;
                            previous.0.push(end);
                        }
                    } else {
                        budget.emit_positions(2)?;
                        output_lines.push(LineString::new(vec![start, end]));
                    }
                }
            }
        }

        let mut uncovered_points = Vec::with_capacity(contact_points.len());
        'point: for point in contact_points {
            for line in &output_lines {
                budget.comparisons(line.coords_count().max(1))?;
                if line.intersects(&point) {
                    continue 'point;
                }
            }
            uncovered_points.push(point);
        }
        Ok(output_lines
            .into_iter()
            .map(Geometry::LineString)
            .chain(uncovered_points.into_iter().map(Geometry::Point))
            .collect())
    }

    fn polygon_polygon_parts(
        a: &Polygon<f64>,
        b: &Polygon<f64>,
        budget: &mut IntersectionBudget,
    ) -> Result<Vec<Geometry<f64>>> {
        let left_boundary: Vec<_> = polygon_boundary(a).collect();
        let right_boundary: Vec<_> = polygon_boundary(b).collect();
        budget.comparisons(left_boundary.len().saturating_mul(right_boundary.len()))?;
        let area =
            MultiPolygon::new(vec![a.clone()]).intersection(&MultiPolygon::new(vec![b.clone()]));
        budget.emit_positions(area.coords_count())?;
        let mut points = Vec::new();
        let mut lines = Vec::new();
        let mut point_keys = HashSet::new();
        let mut line_keys = HashSet::new();
        for first in left_boundary {
            for second in &right_boundary {
                record_line_intersection(
                    first,
                    *second,
                    &mut points,
                    &mut lines,
                    &mut point_keys,
                    &mut line_keys,
                    budget,
                )?;
            }
        }
        let mut uncovered_points = Vec::with_capacity(points.len());
        'point: for point in points {
            for line in &lines {
                budget.comparisons(line.coords_count().max(1))?;
                if line.intersects(&point) {
                    continue 'point;
                }
            }
            uncovered_points.push(point);
        }
        let mut boundary_parts: Vec<_> = lines
            .into_iter()
            .map(Geometry::LineString)
            .chain(uncovered_points.into_iter().map(Geometry::Point))
            .collect();
        let mut uncovered_boundary_parts = Vec::with_capacity(boundary_parts.len());
        'part: for part in boundary_parts.drain(..) {
            for polygon in &area.0 {
                let comparison_cost = match &part {
                    Geometry::Point(_) => polygon.coords_count(),
                    Geometry::LineString(line) => {
                        polygon.coords_count().saturating_mul(line.coords_count())
                    }
                    _ => 1,
                };
                budget.comparisons(comparison_cost.max(1))?;
                let covered = match &part {
                    Geometry::Point(point) => polygon.intersects(point),
                    Geometry::LineString(line) => polygon.relate(line).is_covers(),
                    _ => false,
                };
                if covered {
                    continue 'part;
                }
            }
            uncovered_boundary_parts.push(part);
        }
        Ok(area
            .0
            .into_iter()
            .map(Geometry::Polygon)
            .chain(uncovered_boundary_parts)
            .collect())
    }

    fn atomic_intersection(
        a: &Geometry<f64>,
        b: &Geometry<f64>,
        budget: &mut IntersectionBudget,
    ) -> Result<Vec<Geometry<f64>>> {
        match (a, b) {
            (Geometry::Point(point), other) | (other, Geometry::Point(point)) => {
                budget.comparisons(other.coords_count().max(1))?;
                if other.intersects(point) {
                    budget.emit_positions(1)?;
                    Ok(vec![Geometry::Point(*point)])
                } else {
                    Ok(Vec::new())
                }
            }
            (Geometry::LineString(a), Geometry::LineString(b)) => line_line_parts(a, b, budget),
            (Geometry::LineString(line), Geometry::Polygon(polygon))
            | (Geometry::Polygon(polygon), Geometry::LineString(line)) => {
                line_polygon_parts(line, polygon, budget)
            }
            (Geometry::Polygon(a), Geometry::Polygon(b)) => polygon_polygon_parts(a, b, budget),
            _ => Ok(Vec::new()),
        }
    }

    fn assemble_intersection(
        parts: Vec<Geometry<f64>>,
        budget: &mut IntersectionBudget,
    ) -> Result<Geometry<f64>> {
        let mut keys = HashSet::new();
        let mut points = Vec::new();
        let mut lines = Vec::new();
        let mut polygons = Vec::new();
        let mut positions = 0usize;
        for part in parts {
            let Some(key) = intersection_geometry_key(&part) else {
                continue;
            };
            if keys.insert(key) {
                positions = positions.saturating_add(part.coords_count());
                ensure!(
                    positions <= MAX_GEOMETRY_POSITIONS,
                    "Intersection exceeds the Geometry position limit"
                );
                match part {
                    Geometry::Point(point) => points.push(point),
                    Geometry::LineString(line) => lines.push(line),
                    Geometry::Polygon(polygon) => polygons.push(polygon),
                    _ => unreachable!(),
                }
            }
        }

        if polygons.len() > 1 {
            let polygon_pairs = polygons
                .len()
                .saturating_mul(polygons.len().saturating_sub(1))
                / 2;
            budget.comparisons(polygon_pairs)?;
            polygons = unary_union(polygons.iter()).0;
            ensure!(
                polygons.iter().fold(0usize, |count, polygon| count
                    .saturating_add(polygon.coords_count()))
                    <= MAX_GEOMETRY_POSITIONS,
                "Dissolved intersection exceeds the Geometry position limit"
            );
        }

        let mut uncovered_lines = Vec::with_capacity(lines.len());
        'line: for line in lines {
            for polygon in &polygons {
                budget.comparisons(
                    polygon
                        .coords_count()
                        .saturating_mul(line.coords_count())
                        .max(1),
                )?;
                if polygon.relate(&line).is_covers() {
                    continue 'line;
                }
            }
            uncovered_lines.push(line);
        }

        let mut uncovered_points = Vec::with_capacity(points.len());
        'point: for point in points {
            for polygon in &polygons {
                budget.comparisons(polygon.coords_count().max(1))?;
                if polygon.intersects(&point) {
                    continue 'point;
                }
            }
            for line in &uncovered_lines {
                budget.comparisons(line.coords_count().max(1))?;
                if line.intersects(&point) {
                    continue 'point;
                }
            }
            uncovered_points.push(point);
        }

        let final_positions = polygons
            .iter()
            .fold(0usize, |count, polygon| {
                count.saturating_add(polygon.coords_count())
            })
            .saturating_add(uncovered_lines.iter().fold(0usize, |count, line| {
                count.saturating_add(line.coords_count())
            }))
            .saturating_add(uncovered_points.len());
        ensure!(
            final_positions <= MAX_GEOMETRY_POSITIONS,
            "Intersection exceeds the Geometry position limit"
        );
        let component_count = polygons
            .len()
            .saturating_add(uncovered_lines.len())
            .saturating_add(uncovered_points.len());
        match component_count {
            0 => Ok(Geometry::GeometryCollection(GeometryCollection::empty())),
            1 if !polygons.is_empty() => Ok(Geometry::Polygon(
                polygons.pop().expect("one polygon intersection part"),
            )),
            1 if !uncovered_lines.is_empty() => Ok(Geometry::LineString(
                uncovered_lines.pop().expect("one line intersection part"),
            )),
            1 => Ok(Geometry::Point(
                uncovered_points.pop().expect("one point intersection part"),
            )),
            count if polygons.len() == count => {
                Ok(Geometry::MultiPolygon(MultiPolygon::new(polygons)))
            }
            count if uncovered_lines.len() == count => Ok(Geometry::MultiLineString(
                MultiLineString::new(uncovered_lines),
            )),
            count if uncovered_points.len() == count => {
                Ok(Geometry::MultiPoint(MultiPoint::new(uncovered_points)))
            }
            _ => {
                let members = polygons
                    .into_iter()
                    .map(Geometry::Polygon)
                    .chain(uncovered_lines.into_iter().map(Geometry::LineString))
                    .chain(uncovered_points.into_iter().map(Geometry::Point))
                    .collect();
                Ok(Geometry::GeometryCollection(GeometryCollection::new_from(
                    members,
                )))
            }
        }
    }

    /// Computes intersections for the seven GeoJSON Geometry variants, preserving point and line
    /// results when polygon area overlay alone would return empty.
    pub(crate) fn mixed_dimension_intersection(
        a: Geometry<f64>,
        b: Geometry<f64>,
    ) -> Result<Geometry<f64>> {
        let mut left = Vec::new();
        let mut right = Vec::new();
        atomic_geometries(&a, &mut left);
        atomic_geometries(&b, &mut right);
        ensure!(
            left.len().saturating_mul(right.len()) <= MAX_INTERSECTION_COMPARISONS,
            "Mixed-dimensional intersection has too many component pairs"
        );
        let mut budget = IntersectionBudget::default();
        let mut parts = Vec::new();
        let mut part_positions = 0usize;
        let pair_count = left.len().saturating_mul(right.len());
        let left_positions: Vec<_> = left.iter().map(CoordsIter::coords_count).collect();
        let right_positions: Vec<_> = right.iter().map(CoordsIter::coords_count).collect();
        for start in (0..pair_count).step_by(INTERSECTION_BATCH_SIZE) {
            let end = pair_count.min(start + INTERSECTION_BATCH_SIZE);
            let work = (start..end).fold(0usize, |work, index| {
                work.saturating_add(
                    left_positions[index / right.len()]
                        .saturating_mul(right_positions[index % right.len()])
                        .max(1),
                )
            });
            // Workers share one budget, charging before each bounded action. Batches
            // bound task overhead, and indexed collection preserves component order.
            let results = cpu::map_indices(end - start, work, |offset| {
                let index = start + offset;
                atomic_intersection(
                    &left[index / right.len()],
                    &right[index % right.len()],
                    &mut budget.clone(),
                )
            });
            for pair_parts in results {
                let pair_parts = pair_parts?;
                let pair_positions = pair_parts.iter().fold(0usize, |count, part| {
                    count.saturating_add(part.coords_count())
                });
                part_positions = part_positions.saturating_add(pair_positions);
                ensure!(
                    part_positions <= MAX_GEOMETRY_POSITIONS,
                    "Intersection exceeds the Geometry position limit"
                );
                parts.extend(pair_parts);
            }
        }
        assemble_intersection(parts, &mut budget)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn point(x: f64, y: f64) -> Value {
            json!({"type":"Point","coordinates":[x,y]})
        }

        fn line(coords: Value) -> Value {
            json!({"type":"LineString","coordinates":coords})
        }

        fn square() -> Value {
            json!({"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]]]})
        }

        fn result(operation: AdvancedOperation, inputs: Value, output: &str) -> Value {
            execute(operation, &inputs)
                .unwrap()
                .into_iter()
                .find(|(name, _)| *name == output)
                .unwrap()
                .1
        }

        fn circular_ring(vertices: usize, offset: f64) -> LineString<f64> {
            let mut coordinates: Vec<_> = (0..vertices)
                .map(|index| {
                    let angle = std::f64::consts::TAU * index as f64 / vertices as f64;
                    Coord {
                        x: offset + angle.cos(),
                        y: angle.sin(),
                    }
                })
                .collect();
            coordinates.push(coordinates[0]);
            LineString::new(coordinates)
        }

        fn crossing_line_sets(count: usize) -> (Geometry<f64>, Geometry<f64>) {
            let horizontal = (0..count)
                .map(|index| {
                    LineString::new(
                        (0..=64)
                            .map(|x| Coord {
                                x: x as f64,
                                y: index as f64 + 0.5,
                            })
                            .collect(),
                    )
                })
                .collect();
            let vertical = (0..count)
                .map(|index| {
                    LineString::new(
                        (0..=64)
                            .map(|y| Coord {
                                x: index as f64 + 0.5,
                                y: y as f64,
                            })
                            .collect(),
                    )
                })
                .collect();
            (
                Geometry::MultiLineString(MultiLineString::new(horizontal)),
                Geometry::MultiLineString(MultiLineString::new(vertical)),
            )
        }

        #[tokio::test]
        async fn parallel_ring_checks_and_triangulation_match_serial_results() {
            let ring = circular_ring(512, 0.0);
            let mut crossing = ring.clone();
            crossing.0.swap(64, 320);
            let expected_ring = line_is_ring(&ring).unwrap();
            let expected_crossing = line_is_ring(&crossing).unwrap();
            assert!(expected_ring);
            assert!(!expected_crossing);
            let polygon_set = Geometry::MultiPolygon(MultiPolygon::new(
                (0..4)
                    .map(|index| Polygon::new(circular_ring(128, index as f64 * 4.0), vec![]))
                    .collect(),
            ));
            let expected_triangles = triangulate(polygon_set.clone()).unwrap();
            let (actual_ring, actual_crossing, actual_triangles) = cpu::run(move || {
                Ok((
                    line_is_ring(&ring)?,
                    line_is_ring(&crossing)?,
                    triangulate(polygon_set)?,
                ))
            })
            .await
            .unwrap();
            assert_eq!(actual_ring, expected_ring);
            assert_eq!(actual_crossing, expected_crossing);
            assert_eq!(actual_triangles, expected_triangles);
        }

        #[tokio::test]
        async fn parallel_component_intersections_keep_serial_order_and_aggregate_limits() {
            let (left, right) = crossing_line_sets(12);
            let expected = mixed_dimension_intersection(left.clone(), right.clone()).unwrap();
            assert!(matches!(&expected, Geometry::MultiPoint(points) if points.0.len() == 144));
            let actual = cpu::run(move || mixed_dimension_intersection(left, right))
                .await
                .unwrap();
            assert_eq!(actual, expected);

            let (left, right) = crossing_line_sets(32);
            let serial_error =
                mixed_dimension_intersection(left.clone(), right.clone()).unwrap_err();
            let parallel_error = cpu::run(move || mixed_dimension_intersection(left, right))
                .await
                .unwrap_err();
            assert!(serial_error.to_string().contains("limit"));
            assert_eq!(parallel_error.to_string(), serial_error.to_string());
        }

        #[tokio::test]
        async fn parallel_intersections_share_the_entire_output_budget() {
            let budget = IntersectionBudget::default();
            let counters = budget.counters.clone();
            let accepted = cpu::run(move || {
                Ok(cpu::map_indices(64, MAX_INTERSECTION_COMPARISONS, |_| {
                    budget
                        .clone()
                        .emit_positions(MAX_GEOMETRY_POSITIONS / 32)
                        .is_ok()
                }))
            })
            .await
            .unwrap();
            assert_eq!(accepted.iter().filter(|accepted| **accepted).count(), 32);
            assert_eq!(
                counters.emitted_positions.load(Ordering::Relaxed),
                MAX_GEOMETRY_POSITIONS
            );
        }

        #[test]
        fn ring_and_triangulation_reject_excess_work_before_parallel_allocation() {
            assert!(
                line_is_ring(&circular_ring(2_002, 0.0))
                    .unwrap_err()
                    .to_string()
                    .contains("comparison limit")
            );
            let polygon = Polygon::new(
                LineString::from(vec![
                    (0.0, 0.0),
                    (1.0, 0.0),
                    (1.0, 1.0),
                    (0.0, 1.0),
                    (0.0, 0.0),
                ]),
                vec![],
            );
            let polygons = MultiPolygon::new(vec![polygon; MAX_GEOMETRY_POSITIONS / 8 + 1]);
            assert!(
                triangulate(Geometry::MultiPolygon(polygons))
                    .unwrap_err()
                    .to_string()
                    .contains("position limit")
            );
        }

        #[test]
        fn derived_geometry_handles_empty_and_degenerate_inputs() {
            assert_eq!(
                result(
                    AdvancedOperation::PointOnSurface,
                    json!({"geometry":square()}),
                    "geometry_out"
                )["type"],
                "Point"
            );
            assert_eq!(
                result(
                    AdvancedOperation::Envelope,
                    json!({"geometry":point(2.0, 3.0)}),
                    "geometry_out"
                ),
                point(2.0, 3.0)
            );
            assert_eq!(
                result(
                    AdvancedOperation::MinimumRotatedRectangle,
                    json!({"geometry":line(json!([[0,0],[2,2],[1,1]]))}),
                    "geometry_out"
                )["type"],
                "LineString"
            );
            let closest = result(
                AdvancedOperation::ClosestPoint,
                json!({"geometry":line(json!([[0,0],[4,0]])),"point":point(2.0,3.0)}),
                "geometry_out",
            );
            assert_eq!(closest, point(2.0, 0.0));
        }

        #[test]
        fn line_editing_uses_length_ratios_and_bounded_densification() {
            let source = line(json!([[0, 0], [2, 0], [2, 2]]));
            assert_eq!(
                result(
                    AdvancedOperation::InterpolateLine,
                    json!({"geometry":source,"ratio":0.75}),
                    "geometry_out"
                ),
                point(2.0, 1.0)
            );
            assert_eq!(
                result(
                    AdvancedOperation::LocatePoint,
                    json!({"geometry":source,"point":point(2.0,1.0)}),
                    "ratio"
                ),
                0.75
            );
            assert_eq!(
                result(
                    AdvancedOperation::InterpolateLine,
                    json!({"geometry":source,"ratio":1.0}),
                    "geometry_out"
                ),
                point(2.0, 2.0)
            );
            let substring = result(
                AdvancedOperation::LineSubstring,
                json!({"geometry":source,"start_ratio":0.25,"end_ratio":0.75}),
                "geometry_out",
            );
            assert_eq!(
                substring["coordinates"],
                json!([[1.0, 0.0], [2.0, 0.0], [2.0, 1.0]])
            );
            let closed = line(json!([[0, 0], [2, 0], [2, 2], [0, 0]]));
            assert_eq!(
                result(
                    AdvancedOperation::LineSubstring,
                    json!({"geometry":closed,"start_ratio":0.0,"end_ratio":1.0}),
                    "geometry_out",
                )["coordinates"],
                json!([[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 0.0]])
            );
            let dense = result(
                AdvancedOperation::DensifyPlanar,
                json!({"geometry":line(json!([[0,0],[2,0]])),"max_length":0.75}),
                "geometry_out",
            );
            assert_eq!(dense["coordinates"].as_array().unwrap().len(), 4);
            assert!(
                execute(
                    AdvancedOperation::DensifyPlanar,
                    &json!({"geometry":line(json!([[0,0],[1,0]])),"max_length":0.000001})
                )
                .is_err()
            );
        }

        #[test]
        fn ring_predicate_rejects_crossings_and_line_reverse_is_exact() {
            let ring = line(json!([[0, 0], [2, 0], [2, 2], [0, 2], [0, 0]]));
            assert_eq!(
                result(
                    AdvancedOperation::IsRing,
                    json!({"geometry":ring}),
                    "result"
                ),
                true
            );
            let crossing = line(json!([[0, 0], [2, 2], [0, 2], [2, 0], [0, 0]]));
            assert_eq!(
                result(
                    AdvancedOperation::IsRing,
                    json!({"geometry":crossing}),
                    "result"
                ),
                false
            );
            let reversed = result(
                AdvancedOperation::ReverseLine,
                json!({"geometry":line(json!([[0,0],[1,2],[3,4]]))}),
                "geometry_out",
            );
            assert_eq!(
                reversed["coordinates"],
                json!([[3.0, 4.0], [1.0, 2.0], [0.0, 0.0]])
            );
        }

        #[test]
        fn geodesic_navigation_and_antimeridian_split_are_explicit() {
            let bearing = result(
                AdvancedOperation::Bearing,
                json!({"a":point(0.0,0.0),"b":point(1.0,0.0)}),
                "bearing",
            )
            .as_f64()
            .unwrap();
            assert!((bearing - 90.0).abs() < 1e-9);
            let destination = result(
                AdvancedOperation::Destination,
                json!({"origin":point(0.0,0.0),"bearing":90.0,"distance":111_319.490_793}),
                "geometry_out",
            );
            assert!((destination["coordinates"][0].as_f64().unwrap() - 1.0).abs() < 1e-8);
            let split = result(
                AdvancedOperation::SplitAntimeridian,
                json!({"geometry":line(json!([[170,0],[-170,10]]))}),
                "geometry_out",
            );
            assert_eq!(split["coordinates"].as_array().unwrap().len(), 2);
            assert_eq!(split["coordinates"][0][1], json!([180.0, 5.0]));
            assert_eq!(split["coordinates"][1][0], json!([-180.0, 5.0]));

            let crossing_from_boundary = result(
                AdvancedOperation::SplitAntimeridian,
                json!({"geometry":line(json!([[-170,0],[-180,1],[170,2]]))}),
                "geometry_out",
            );
            assert_eq!(
                crossing_from_boundary["coordinates"],
                json!([[[-170.0, 0.0], [-180.0, 1.0]], [[180.0, 1.0], [170.0, 2.0]]])
            );

            let crossing_to_boundary = result(
                AdvancedOperation::SplitAntimeridian,
                json!({"geometry":line(json!([[170,0],[-180,1],[-170,2]]))}),
                "geometry_out",
            );
            assert_eq!(
                crossing_to_boundary["coordinates"],
                json!([[[170.0, 0.0], [180.0, 1.0]], [[-180.0, 1.0], [-170.0, 2.0]]])
            );

            let equivalent_boundary_endpoints = result(
                AdvancedOperation::SplitAntimeridian,
                json!({"geometry":line(json!([[180,0],[-180,1]]))}),
                "geometry_out",
            );
            assert_eq!(
                equivalent_boundary_endpoints["coordinates"],
                json!([[[180.0, 0.0], [180.0, 1.0]]])
            );
        }

        #[test]
        fn transforms_triangulation_and_smoothing_validate_results() {
            let translated = result(
                AdvancedOperation::Translate,
                json!({"geometry":point(1.0,2.0),"longitude_offset":3.0,"latitude_offset":4.0}),
                "geometry_out",
            );
            assert_eq!(translated, point(4.0, 6.0));
            let triangles = result(
                AdvancedOperation::Triangulate,
                json!({"geometry":square()}),
                "geometry_out",
            );
            assert_eq!(triangles["type"], "GeometryCollection");
            assert_eq!(triangles["geometries"].as_array().unwrap().len(), 2);
            let smoothed = result(
                AdvancedOperation::Smooth,
                json!({"geometry":line(json!([[0,0],[1,1],[2,0]])),"iterations":1}),
                "geometry_out",
            );
            assert!(smoothed["coordinates"].as_array().unwrap().len() > 3);

            let collection = json!({
                "type":"GeometryCollection",
                "geometries":[line(json!([[0,0],[1,1],[2,0]])), point(3.0,4.0)]
            });
            let smoothed_collection = result(
                AdvancedOperation::Smooth,
                json!({"geometry":collection,"iterations":1}),
                "geometry_out",
            );
            assert_eq!(smoothed_collection["type"], "GeometryCollection");
            assert_eq!(
                smoothed_collection["geometries"][0]["coordinates"]
                    .as_array()
                    .unwrap()
                    .len(),
                6
            );
            assert_eq!(smoothed_collection["geometries"][1], point(3.0, 4.0));

            let point_set = json!({"type":"MultiPoint","coordinates":[[0,0],[1,1]]});
            assert_eq!(
                result(
                    AdvancedOperation::Smooth,
                    json!({"geometry":point_set,"iterations":16}),
                    "geometry_out",
                ),
                json!({"type":"MultiPoint","coordinates":[[0.0,0.0],[1.0,1.0]]})
            );
        }

        #[test]
        fn mixed_intersection_preserves_crossings_overlaps_and_polygon_touches() {
            let crossing = mixed_dimension_intersection(
                checked_geometry(&line(json!([[0, 0], [2, 2]]))).unwrap(),
                checked_geometry(&line(json!([[0, 2], [2, 0]]))).unwrap(),
            )
            .unwrap();
            assert_eq!(encoded_geometry(crossing).unwrap(), point(1.0, 1.0));

            let clipped = mixed_dimension_intersection(
                checked_geometry(&line(json!([[-1, 2], [5, 2]]))).unwrap(),
                checked_geometry(&square()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                encoded_geometry(clipped).unwrap()["coordinates"],
                json!([[0.0, 2.0], [4.0, 2.0]])
            );

            let touching =
                json!({"type":"Polygon","coordinates":[[[4,0],[8,0],[8,4],[4,4],[4,0]]]});
            let shared_edge = mixed_dimension_intersection(
                checked_geometry(&square()).unwrap(),
                checked_geometry(&touching).unwrap(),
            )
            .unwrap();
            assert!(matches!(shared_edge, Geometry::LineString(_)));
        }

        #[test]
        fn mixed_intersection_deduplicates_dissolves_and_uses_homogeneous_outputs() {
            let points = json!({
                "type":"MultiPoint",
                "coordinates":[[1,1],[3,3],[5,5]]
            });
            let point_result = mixed_dimension_intersection(
                checked_geometry(&points).unwrap(),
                checked_geometry(&square()).unwrap(),
            )
            .unwrap();
            assert!(matches!(point_result, Geometry::MultiPoint(_)));

            let lines = json!({
                "type":"MultiLineString",
                "coordinates":[[[-1,1],[1,1]],[[-1,3],[1,3]]]
            });
            let line_result = mixed_dimension_intersection(
                checked_geometry(&lines).unwrap(),
                checked_geometry(&square()).unwrap(),
            )
            .unwrap();
            assert!(matches!(line_result, Geometry::MultiLineString(_)));

            let duplicate_lines = json!({
                "type":"GeometryCollection",
                "geometries":[
                    line(json!([[0,0],[2,0]])),
                    line(json!([[2,0],[0,0]]))
                ]
            });
            let deduplicated = mixed_dimension_intersection(
                checked_geometry(&duplicate_lines).unwrap(),
                checked_geometry(&line(json!([[0, 0], [2, 0]]))).unwrap(),
            )
            .unwrap();
            assert!(matches!(deduplicated, Geometry::LineString(_)));

            let overlapping_polygons = json!({
                "type":"GeometryCollection",
                "geometries":[
                    {"type":"Polygon","coordinates":[[[0,0],[3,0],[3,3],[0,3],[0,0]]]},
                    {"type":"Polygon","coordinates":[[[2,0],[5,0],[5,3],[2,3],[2,0]]]}
                ]
            });
            let mask = json!({
                "type":"Polygon",
                "coordinates":[[[-1,-1],[6,-1],[6,4],[-1,4],[-1,-1]]]
            });
            let dissolved = mixed_dimension_intersection(
                checked_geometry(&overlapping_polygons).unwrap(),
                checked_geometry(&mask).unwrap(),
            )
            .unwrap();
            assert!(matches!(&dissolved, Geometry::Polygon(_)));
            assert!(encoded_geometry(dissolved).is_ok());
        }
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn advanced_nodes_have_unique_ids_and_geometry_receivers() {
        let operations = [
            AdvancedOperation::PointOnSurface,
            AdvancedOperation::ClosestPoint,
            AdvancedOperation::ConcaveHull,
            AdvancedOperation::MinimumRotatedRectangle,
            AdvancedOperation::Envelope,
            AdvancedOperation::SimplifyVw,
            AdvancedOperation::SimplifyVwPreserve,
            AdvancedOperation::RemoveRepeatedPoints,
            AdvancedOperation::ReverseLine,
            AdvancedOperation::IsClosed,
            AdvancedOperation::IsRing,
            AdvancedOperation::InterpolateLine,
            AdvancedOperation::LocatePoint,
            AdvancedOperation::LineSubstring,
            AdvancedOperation::DensifyPlanar,
            AdvancedOperation::DensifyGeodesic,
            AdvancedOperation::Bearing,
            AdvancedOperation::Destination,
            AdvancedOperation::GeodesicInterpolate,
            AdvancedOperation::Translate,
            AdvancedOperation::Rotate,
            AdvancedOperation::Scale,
            AdvancedOperation::Skew,
            AdvancedOperation::AffineTransform,
            AdvancedOperation::Triangulate,
            AdvancedOperation::Smooth,
            AdvancedOperation::SplitAntimeridian,
        ];
        let definitions: Vec<_> = operations.into_iter().map(definition).collect();
        let ids: std::collections::HashSet<_> =
            definitions.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(ids.len(), definitions.len());
        for node in definitions {
            let first_input = node
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Input)
                .min_by_key(|pin| pin.index)
                .unwrap();
            assert_eq!(first_input.data_type, VariableType::Geometry);
        }
    }
}
