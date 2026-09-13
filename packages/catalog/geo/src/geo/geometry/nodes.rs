use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    geometry::{GeometryKind, marker},
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Operation {
    MakePoint,
    FromGeoJson,
    ToGeoJson,
    FromWkt,
    ToWkt,
    FromWkb,
    ToWkb,
    FromCoordinate,
    ToCoordinate,
    FromBoundary,
    FromPolygons,
    FromRoute,
    ToRoute,
    FromLocation,
    Type,
    X,
    Y,
    Bounds,
    NumPoints,
    Contains,
    Intersects,
    Within,
    Intersection,
    Centroid,
    ConvexHull,
    Simplify,
    PlanarDistance,
    GeodesicDistance,
    PlanarLength,
    GeodesicLength,
    PlanarArea,
    GeodesicArea,
    Cast(GeometryKind),
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
fn data_input<'a>(node: &'a mut Node, name: &str, ty: VariableType) -> &'a mut Pin {
    node.add_input_pin(name, name, name, ty)
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
        MakePoint => (
            "geometry_make_point",
            "makePoint",
            "Make Point",
            "Creates a WGS 84 Point from longitude and latitude. Both coordinates must be finite and within geographic bounds.",
        ),
        FromGeoJson => (
            "geometry_from_geojson",
            "fromGeoJson",
            "Parse GeoJSON Geometry",
            "Parses a GeoJSON geometry object, validates its two-dimensional WGS 84 coordinates and normalizes ring winding. Retains bbox and foreign members. Feature wrappers require extraction.",
        ),
        ToGeoJson => (
            "geometry_to_geojson",
            "toGeoJson",
            "Geometry to GeoJSON",
            "Writes a validated geometry as GeoJSON text, retaining bbox and foreign members and normalizing ring winding.",
        ),
        FromWkt => (
            "geometry_from_wkt",
            "fromWkt",
            "Parse WGS 84 WKT",
            "Parses two-dimensional WKT. Calling this node asserts coordinates are WGS 84 longitude/latitude; it does not transform a projected CRS. Empty scalar geometries are rejected.",
        ),
        ToWkt => (
            "geometry_to_wkt",
            "toWkt",
            "Geometry to WKT",
            "Writes two-dimensional WKT. WKT omits GeoJSON bbox and foreign members; keep application properties in a surrounding Struct.",
        ),
        FromWkb => (
            "geometry_from_wkb",
            "fromWkb",
            "Parse WGS 84 WKB",
            "Parses two-dimensional WKB bytes. Calling this node asserts WGS 84 longitude/latitude. Unsupported dimensions, SRIDs and out-of-range coordinates are rejected.",
        ),
        ToWkb => (
            "geometry_to_wkb",
            "toWkb",
            "Geometry to WKB",
            "Writes two-dimensional WKB bytes. WKB omits GeoJSON bbox and foreign members; keep application properties in a surrounding Struct.",
        ),
        FromCoordinate => (
            "geometry_from_legacy_coordinate",
            "fromLegacyCoordinate",
            "Legacy Coordinate to Point",
            "Converts the existing GeoCoordinate latitude/longitude object to a Geometry Point without swapping the axes.",
        ),
        ToCoordinate => (
            "geometry_to_legacy_coordinate",
            "toLegacyCoordinate",
            "Point to Legacy Coordinate",
            "Converts a Geometry Point into the existing GeoCoordinate shape used by H3, routing, search and map nodes.",
        ),
        FromBoundary => (
            "geometry_from_legacy_boundary",
            "fromLegacyBoundary",
            "Legacy Boundary to Geometry",
            "Converts the coordinate vector emitted by H3 Cell Boundary into Geometry. Closes the ring, validates topology, and returns a split MultiPolygon for a transmeridian cell.",
        ),
        FromPolygons => (
            "geometry_from_legacy_polygons",
            "fromLegacyPolygons",
            "Legacy H3 Polygons to MultiPolygon",
            "Converts the existing H3 polygon vector, preserving exterior and interior rings, closing each ring, and splitting transmeridian outlines.",
        ),
        FromRoute => (
            "geometry_from_legacy_route",
            "fromLegacyRoute",
            "Extract Legacy Route Geometry",
            "Extracts a LineString from RouteResult.geometry.points or RouteGeometry.points. Returns the original wrapper unchanged.",
        ),
        ToRoute => (
            "geometry_to_legacy_route",
            "toLegacyRoute",
            "Update Legacy Route Geometry",
            "Updates only the points of an existing RouteResult or RouteGeometry with a LineString, retaining route metadata and other fields.",
        ),
        FromLocation => (
            "geometry_from_legacy_location",
            "fromLegacyLocation",
            "Extract Legacy Location Point",
            "Extracts a Point from a search result or waypoint coordinate. Returns the original rich location wrapper unchanged.",
        ),
        Type => (
            "geometry_type",
            "type",
            "Geometry Type",
            "Returns the GeoJSON geometry type name.",
        ),
        X => (
            "geometry_x",
            "x",
            "Point Longitude",
            "Returns the Point x coordinate, longitude in degrees.",
        ),
        Y => (
            "geometry_y",
            "y",
            "Point Latitude",
            "Returns the Point y coordinate, latitude in degrees.",
        ),
        Bounds => (
            "geometry_bounds",
            "bounds",
            "Geometry Bounds",
            "Returns minimum and maximum longitude and latitude using a planar coordinate envelope. Empty geometries have no bounds.",
        ),
        NumPoints => (
            "geometry_num_points",
            "numPoints",
            "Geometry Position Count",
            "Counts coordinate positions, including closing polygon positions and recursive collection members.",
        ),
        Contains => (
            "geometry_contains",
            "contains",
            "Geometry Contains (Planar)",
            "Tests whether geometry A contains B in the longitude/latitude coordinate plane. A point on a polygon boundary is not contained.",
        ),
        Intersects => (
            "geometry_intersects",
            "intersects",
            "Geometry Intersects (Planar)",
            "Tests whether geometries share any point in the longitude/latitude coordinate plane, including boundary touches.",
        ),
        Within => (
            "geometry_within",
            "within",
            "Geometry Within (Planar)",
            "Tests whether geometry B contains A in the longitude/latitude coordinate plane.",
        ),
        Intersection => (
            "geometry_intersection",
            "intersection",
            "Geometry Intersection (Planar)",
            "Constructs the intersection of any Geometry inputs in the longitude/latitude coordinate plane. Preserves point, line and polygon results; mixed results use a GeometryCollection.",
        ),
        Centroid => (
            "geometry_centroid",
            "centroid",
            "Geometry Centroid (Planar)",
            "Computes the centroid in the longitude/latitude coordinate plane. Empty geometries have no centroid.",
        ),
        ConvexHull => (
            "geometry_convex_hull",
            "convexHull",
            "Geometry Convex Hull (Planar)",
            "Computes the smallest convex geometry containing the input in the longitude/latitude coordinate plane. A lower-dimensional input produces a Point or LineString.",
        ),
        Simplify => (
            "geometry_simplify",
            "simplify",
            "Simplify Geometry (Planar)",
            "Simplifies lines and polygon rings with a nonnegative tolerance in coordinate degrees. Validates the result and rejects a topology-breaking result.",
        ),
        PlanarDistance => (
            "geometry_planar_distance",
            "planarDistance",
            "Geometry Distance (Degrees)",
            "Computes the shortest planar distance in coordinate degrees. This longitude/latitude plane does not wrap at the antimeridian. Empty inputs are rejected.",
        ),
        GeodesicDistance => (
            "geometry_geodesic_distance",
            "geodesicDistance",
            "Point Distance (Meters)",
            "Computes the WGS 84 ellipsoidal geodesic distance between two Points in meters, including antimeridian crossings.",
        ),
        PlanarLength => (
            "geometry_planar_length",
            "planarLength",
            "Geometry Length (Degrees)",
            "Sums line lengths and polygon ring perimeters in planar coordinate degrees. Points contribute zero.",
        ),
        GeodesicLength => (
            "geometry_geodesic_length",
            "geodesicLength",
            "Geometry Length (Meters)",
            "Sums WGS 84 ellipsoidal geodesic segment lengths in meters, including polygon exterior and interior ring perimeters. Points contribute zero.",
        ),
        PlanarArea => (
            "geometry_planar_area",
            "planarArea",
            "Geometry Area (Square Degrees)",
            "Computes area in square coordinate degrees, subtracting polygon holes and summing collection members. Points and lines contribute zero.",
        ),
        GeodesicArea => (
            "geometry_geodesic_area",
            "geodesicArea",
            "Geometry Area (Square Meters)",
            "Computes WGS 84 ellipsoidal area in square meters, subtracting polygon holes and summing collection members. Points and lines contribute zero. Each polygon must describe a region smaller than half the Earth.",
        ),
        Cast(GeometryKind::Point) => (
            "geometry_cast_point",
            "castPoint",
            "Validate Point",
            "Validates a geometry as Point and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
        Cast(GeometryKind::LineString) => (
            "geometry_cast_line_string",
            "castLineString",
            "Validate LineString",
            "Validates a geometry as LineString and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
        Cast(GeometryKind::Polygon) => (
            "geometry_cast_polygon",
            "castPolygon",
            "Validate Polygon",
            "Validates a geometry as Polygon and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
        Cast(GeometryKind::MultiPoint) => (
            "geometry_cast_multi_point",
            "castMultiPoint",
            "Validate MultiPoint",
            "Validates a geometry as MultiPoint and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
        Cast(GeometryKind::MultiLineString) => (
            "geometry_cast_multi_line_string",
            "castMultiLineString",
            "Validate MultiLineString",
            "Validates a geometry as MultiLineString and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
        Cast(GeometryKind::MultiPolygon) => (
            "geometry_cast_multi_polygon",
            "castMultiPolygon",
            "Validate MultiPolygon",
            "Validates a geometry as MultiPolygon and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
        Cast(GeometryKind::GeometryCollection) => (
            "geometry_cast_geometry_collection",
            "castGeometryCollection",
            "Validate GeometryCollection",
            "Validates a geometry as GeometryCollection and returns it with an explicit subtype. Incompatible values fail the node.",
        ),
    };
    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");
    match operation {
        MakePoint => {
            data_input(&mut node, "longitude", VariableType::Float);
            data_input(&mut node, "latitude", VariableType::Float);
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        Cast(kind) => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, Some(kind));
        }
        FromGeoJson | FromWkt => {
            data_input(&mut node, "text", VariableType::String);
            geometry_output(&mut node, None);
        }
        ToGeoJson | ToWkt => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "text",
                VariableType::String,
                "Serialized geometry",
            );
        }
        FromWkb => {
            data_input(&mut node, "bytes", VariableType::Byte).set_value_type(ValueType::Array);
            geometry_output(&mut node, None);
        }
        ToWkb => {
            geometry_input(&mut node, "geometry", None);
            data_output(&mut node, "bytes", VariableType::Byte, "WKB byte sequence")
                .set_value_type(ValueType::Array);
        }
        FromCoordinate => {
            data_input(&mut node, "coordinate", VariableType::Struct)
                .set_schema::<crate::geo::GeoCoordinate>();
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        ToCoordinate => {
            geometry_input(&mut node, "geometry", Some(GeometryKind::Point));
            data_output(
                &mut node,
                "coordinate",
                VariableType::Struct,
                "Legacy latitude/longitude object",
            )
            .set_schema::<crate::geo::GeoCoordinate>();
        }
        FromBoundary => {
            // The legacy H3 boundary payload is a vector despite its scalar Struct pin declaration.
            data_input(&mut node, "boundary", VariableType::Struct)
                .set_schema::<crate::geo::GeoCoordinate>();
            geometry_output(&mut node, None);
        }
        FromPolygons => {
            data_input(&mut node, "polygons", VariableType::Struct)
                .set_schema::<crate::geo::h3::cells_to_multi_polygon::Polygon>();
            geometry_output(&mut node, Some(GeometryKind::MultiPolygon));
        }
        FromRoute => {
            data_input(&mut node, "route", VariableType::Struct);
            geometry_output(&mut node, Some(GeometryKind::LineString));
            data_output(
                &mut node,
                "route_out",
                VariableType::Struct,
                "Original route wrapper",
            );
        }
        ToRoute => {
            data_input(&mut node, "route", VariableType::Struct);
            geometry_input(&mut node, "geometry", Some(GeometryKind::LineString));
            data_output(
                &mut node,
                "route_out",
                VariableType::Struct,
                "Route with updated points and retained metadata",
            );
        }
        FromLocation => {
            data_input(&mut node, "location", VariableType::Struct);
            geometry_output(&mut node, Some(GeometryKind::Point));
            data_output(
                &mut node,
                "location_out",
                VariableType::Struct,
                "Original location wrapper",
            );
        }
        Type => {
            geometry_input(&mut node, "geometry", None);
            data_output(&mut node, "type", VariableType::String, "GeoJSON type name");
        }
        X | Y => {
            geometry_input(&mut node, "geometry", Some(GeometryKind::Point));
            data_output(
                &mut node,
                "value",
                VariableType::Float,
                "Coordinate in degrees",
            );
        }
        Bounds => {
            geometry_input(&mut node, "geometry", None);
            for name in [
                "min_longitude",
                "min_latitude",
                "max_longitude",
                "max_latitude",
            ] {
                data_output(
                    &mut node,
                    name,
                    VariableType::Float,
                    "Coordinate in degrees",
                );
            }
        }
        NumPoints => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "count",
                VariableType::Integer,
                "Number of positions, including closing ring positions",
            );
        }
        Contains | Intersects | Within => {
            geometry_input(&mut node, "a", None);
            geometry_input(&mut node, "b", None);
            data_output(
                &mut node,
                "result",
                VariableType::Boolean,
                "Planar predicate result",
            );
        }
        Intersection => {
            geometry_input(&mut node, "a", None);
            geometry_input(&mut node, "b", None);
            geometry_output(&mut node, None);
        }
        Centroid => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, Some(GeometryKind::Point));
        }
        ConvexHull => {
            geometry_input(&mut node, "geometry", None);
            geometry_output(&mut node, None);
        }
        Simplify => {
            geometry_input(&mut node, "geometry", None);
            data_input(&mut node, "tolerance", VariableType::Float);
            geometry_output(&mut node, None);
        }
        PlanarDistance | GeodesicDistance => {
            let kind = matches!(operation, GeodesicDistance).then_some(GeometryKind::Point);
            geometry_input(&mut node, "a", kind);
            geometry_input(&mut node, "b", kind);
            data_output(
                &mut node,
                "distance",
                VariableType::Float,
                if kind.is_some() {
                    "Distance in meters"
                } else {
                    "Planar distance in coordinate degrees"
                },
            );
        }
        PlanarLength | GeodesicLength => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "length",
                VariableType::Float,
                if matches!(operation, GeodesicLength) {
                    "Length in meters"
                } else {
                    "Planar length in coordinate degrees"
                },
            );
        }
        PlanarArea | GeodesicArea => {
            geometry_input(&mut node, "geometry", None);
            data_output(
                &mut node,
                "area",
                VariableType::Float,
                if matches!(operation, GeodesicArea) {
                    "Area in square meters"
                } else {
                    "Planar area in square coordinate degrees"
                },
            );
        }
    }
    if let Some(pin) = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == flow_like::flow::pin::PinType::Input)
        .min_by_key(|pin| pin.index)
        .filter(|pin| pin.data_type == VariableType::Geometry)
        .map(|pin| pin.name.clone())
    {
        node.set_receiver(&pin);
    }
    node
}

#[cfg(feature = "execute")]
async fn run_operation(
    operation: Operation,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    let mut values = flow_like_types::json::Map::new();
    for pin in definition(operation)
        .pins
        .values()
        .filter(|pin| pin.pin_type == flow_like::flow::pin::PinType::Input)
    {
        values.insert(
            pin.name.clone(),
            context
                .evaluate_pin::<flow_like_types::Value>(&pin.name)
                .await?,
        );
    }
    let outputs = super::cpu::run(move || {
        super::operations::execute(operation, &flow_like_types::Value::Object(values))
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
pub struct GeometryMakePointNode;
implement_node!(GeometryMakePointNode, Operation::MakePoint);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromGeoJsonNode;
implement_node!(GeometryFromGeoJsonNode, Operation::FromGeoJson);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToGeoJsonNode;
implement_node!(GeometryToGeoJsonNode, Operation::ToGeoJson);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromWktNode;
implement_node!(GeometryFromWktNode, Operation::FromWkt);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToWktNode;
implement_node!(GeometryToWktNode, Operation::ToWkt);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromWkbNode;
implement_node!(GeometryFromWkbNode, Operation::FromWkb);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToWkbNode;
implement_node!(GeometryToWkbNode, Operation::ToWkb);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromCoordinateNode;
implement_node!(GeometryFromCoordinateNode, Operation::FromCoordinate);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToCoordinateNode;
implement_node!(GeometryToCoordinateNode, Operation::ToCoordinate);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromBoundaryNode;
implement_node!(GeometryFromBoundaryNode, Operation::FromBoundary);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromPolygonsNode;
implement_node!(GeometryFromPolygonsNode, Operation::FromPolygons);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromRouteNode;
implement_node!(GeometryFromRouteNode, Operation::FromRoute);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryToRouteNode;
implement_node!(GeometryToRouteNode, Operation::ToRoute);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromLocationNode;
implement_node!(GeometryFromLocationNode, Operation::FromLocation);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryTypeNode;
implement_node!(GeometryTypeNode, Operation::Type);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryXNode;
implement_node!(GeometryXNode, Operation::X);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryYNode;
implement_node!(GeometryYNode, Operation::Y);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryBoundsNode;
implement_node!(GeometryBoundsNode, Operation::Bounds);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryNumPointsNode;
implement_node!(GeometryNumPointsNode, Operation::NumPoints);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryContainsNode;
implement_node!(GeometryContainsNode, Operation::Contains);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryIntersectsNode;
implement_node!(GeometryIntersectsNode, Operation::Intersects);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryWithinNode;
implement_node!(GeometryWithinNode, Operation::Within);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryIntersectionNode;
implement_node!(GeometryIntersectionNode, Operation::Intersection);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCentroidNode;
implement_node!(GeometryCentroidNode, Operation::Centroid);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryConvexHullNode;
implement_node!(GeometryConvexHullNode, Operation::ConvexHull);

#[crate::register_node]
#[derive(Default)]
pub struct GeometrySimplifyNode;
implement_node!(GeometrySimplifyNode, Operation::Simplify);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPlanarDistanceNode;
implement_node!(GeometryPlanarDistanceNode, Operation::PlanarDistance);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicDistanceNode;
implement_node!(GeometryGeodesicDistanceNode, Operation::GeodesicDistance);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPlanarLengthNode;
implement_node!(GeometryPlanarLengthNode, Operation::PlanarLength);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicLengthNode;
implement_node!(GeometryGeodesicLengthNode, Operation::GeodesicLength);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryPlanarAreaNode;
implement_node!(GeometryPlanarAreaNode, Operation::PlanarArea);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicAreaNode;
implement_node!(GeometryGeodesicAreaNode, Operation::GeodesicArea);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastPointNode;
implement_node!(GeometryCastPointNode, Operation::Cast(GeometryKind::Point));

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastLineStringNode;
implement_node!(
    GeometryCastLineStringNode,
    Operation::Cast(GeometryKind::LineString)
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastPolygonNode;
implement_node!(
    GeometryCastPolygonNode,
    Operation::Cast(GeometryKind::Polygon)
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastMultiPointNode;
implement_node!(
    GeometryCastMultiPointNode,
    Operation::Cast(GeometryKind::MultiPoint)
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastMultiLineStringNode;
implement_node!(
    GeometryCastMultiLineStringNode,
    Operation::Cast(GeometryKind::MultiLineString)
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastMultiPolygonNode;
implement_node!(
    GeometryCastMultiPolygonNode,
    Operation::Cast(GeometryKind::MultiPolygon)
);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryCastGeometryCollectionNode;
implement_node!(
    GeometryCastGeometryCollectionNode,
    Operation::Cast(GeometryKind::GeometryCollection)
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_nodes_register_fixed_subtypes_and_retain_legacy_boundaries() {
        let catalog = crate::get_catalog();
        let nodes: Vec<Node> = catalog.iter().map(|logic| logic.get_node()).collect();
        let ids: std::collections::HashSet<_> =
            nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(ids.len(), nodes.len());
        for node in nodes
            .iter()
            .filter(|node| node.name.starts_with("geometry_"))
        {
            let names: std::collections::HashSet<_> =
                node.pins.values().map(|pin| &pin.name).collect();
            assert_eq!(
                names.len(),
                node.pins.len(),
                "{} has ambiguous pin names",
                node.name
            );
        }
        assert_eq!(
            nodes
                .iter()
                .filter(|node| node.name.starts_with("geometry_"))
                .count(),
            130,
            "the generated registry must include every Geometry node"
        );
        for kind in GeometryKind::ALL {
            let cast = definition(Operation::Cast(kind));
            assert!(ids.contains(cast.name.as_str()));
            let input = cast
                .pins
                .values()
                .find(|pin| pin.pin_type == flow_like::flow::pin::PinType::Input)
                .unwrap();
            let output = cast
                .pins
                .values()
                .find(|pin| pin.pin_type == flow_like::flow::pin::PinType::Output)
                .unwrap();
            assert_eq!(input.data_type, VariableType::Geometry);
            assert_eq!(input.schema, None);
            assert_eq!(output.schema.as_deref(), Some(marker(kind)));
        }
        let point = definition(Operation::MakePoint);
        assert!(point.pins.values().all(|pin| pin.default_value.is_none()));
        let old = nodes
            .iter()
            .find(|node| node.name == "h3_cell_to_boundary")
            .unwrap();
        let old_pin = old
            .pins
            .values()
            .find(|pin| pin.name == "boundary")
            .unwrap();
        let adapter = definition(Operation::FromBoundary);
        let input = adapter
            .pins
            .values()
            .find(|pin| pin.name == "boundary")
            .unwrap();
        assert_eq!(old_pin.data_type, VariableType::Struct);
        assert_eq!(old_pin.value_type, input.value_type);
        assert_eq!(old_pin.schema, input.schema);
    }

    #[cfg(feature = "execute")]
    mod execution {
        use super::*;
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{LogLevel, internal_node::InternalNode, internal_pin::InternalPin},
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState},
            utils::http::HTTPClient,
        };
        use flow_like_types::{
            json::json,
            sync::{Mutex, RwLock},
        };
        use std::sync::{Arc, Weak};

        async fn execution_context(logic: Arc<dyn NodeLogic>) -> ExecutionContext {
            let node = logic.get_node();
            let mut pins = AHashMap::new();
            let mut names = AHashMap::<String, Vec<Arc<InternalPin>>>::new();
            for pin in node.pins.values() {
                let internal = Arc::new(InternalPin::new(pin, false));
                names
                    .entry(pin.name.clone())
                    .or_default()
                    .push(internal.clone());
                pins.insert(pin.id.clone(), internal);
            }
            let current = Arc::new(InternalNode::new(node, pins, logic, names));
            for pin in current.pins.iter() {
                pin.init_node(Arc::downgrade(&current));
                pin.init_connected_to(Vec::new());
                pin.init_depends_on(Vec::new());
            }
            ExecutionContext::new(
                Arc::new(AHashMap::from_iter([(
                    current.node_id().to_owned(),
                    current.clone(),
                )])),
                &Weak::new(),
                &Arc::new(FlowLikeState::new(
                    FlowLikeConfig::new(),
                    HTTPClient::new_without_refetch(),
                )),
                &current,
                &Arc::new(Mutex::new(AHashMap::new())),
                &Arc::new(RwLock::new(AHashMap::new())),
                LogLevel::Debug,
                ExecutionStage::Dev,
                Arc::new(Profile::default()),
                None,
                Arc::new(RwLock::new(Vec::new())),
                None,
                None,
                Arc::new(AHashMap::new()),
                None,
            )
            .await
        }

        async fn run_concurrent_geometry_nodes(flavor: tokio::runtime::RuntimeFlavor) {
            use super::super::super::linear_analysis::GeometryHausdorffDistanceNode;

            let flavor = &flavor;
            let logics: [Arc<dyn NodeLogic>; 2] = [
                Arc::new(GeometryPlanarDistanceNode),
                Arc::new(GeometryHausdorffDistanceNode),
            ];
            let jobs = (0..16).map(|index| {
                let logic = logics[index % logics.len()].clone();
                async move {
                    let mut context = execution_context(logic.clone()).await;
                    let points = |y: f64| {
                        json!({
                            "type":"MultiPoint",
                            "coordinates":(0..192)
                                .map(|position| [position as f64 / 100.0,y])
                                .collect::<Vec<_>>()
                        })
                    };
                    let expected = index as f64 + 1.0;
                    context.set_pin_value("a", points(0.0)).await.unwrap();
                    context.set_pin_value("b", points(expected)).await.unwrap();
                    logic.run(&mut context).await.unwrap();
                    assert_eq!(
                        context.evaluate_pin::<f64>("distance").await.unwrap(),
                        expected
                    );

                    // Invalid pin values must fail before computation and remain ordinary errors.
                    let a_pin = context.get_pin_by_name("a").await.unwrap();
                    context.override_pin_value(
                        &a_pin.id,
                        json!({"type":"Point","coordinates":[181.0,0.0]}),
                    );
                    let error = logic.run(&mut context).await.unwrap_err().to_string();
                    assert!(error.contains("longitude"), "{error}");
                    assert!(!error.contains("panicked"), "{error}");

                    // Empty geometry passes pin validation and fails inside the CPU worker.
                    context
                        .set_pin_value("a", json!({"type":"MultiPoint","coordinates":[]}))
                        .await
                        .unwrap();
                    let error = logic.run(&mut context).await.unwrap_err().to_string();
                    assert!(error.contains("Empty geometries"), "{error}");
                    assert!(!error.contains("panicked"), "{error}");

                    context.set_pin_value("a", points(0.0)).await.unwrap();
                    context
                        .set_pin_value("b", points(expected + 1.0))
                        .await
                        .unwrap();
                    logic.run(&mut context).await.unwrap();
                    assert_eq!(
                        context.evaluate_pin::<f64>("distance").await.unwrap(),
                        expected + 1.0
                    );
                    assert_eq!(&tokio::runtime::Handle::current().runtime_flavor(), flavor);
                }
            });
            futures::future::join_all(jobs).await;
        }

        #[tokio::test(flavor = "current_thread")]
        async fn geometry_node_pins_and_errors_work_on_current_thread_tokio() {
            run_concurrent_geometry_nodes(tokio::runtime::RuntimeFlavor::CurrentThread).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn geometry_node_pins_and_errors_work_on_multithread_tokio() {
            run_concurrent_geometry_nodes(tokio::runtime::RuntimeFlavor::MultiThread).await;
        }
    }
}
