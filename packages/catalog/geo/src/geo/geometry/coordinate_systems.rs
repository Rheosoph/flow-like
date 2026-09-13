//! Explicit coordinate-system boundaries and geodesic construction for Geometry values.

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
enum Operation {
    ToWebMercator,
    FromWebMercator,
    WrapLongitude,
    GeodesicCircle,
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

fn definition(operation: Operation) -> Node {
    use Operation::*;
    let (id, alias, title, description) = match operation {
        ToWebMercator => (
            "geometry_to_web_mercator",
            "toWebMercator",
            "Geometry to Web Mercator",
            "Projects WGS 84 positions to EPSG:3857 meters. Latitude must be within the Web Mercator bound of +/-85.0511287798066 degrees; positions nearer a pole have no EPSG:3857 representation and are rejected. The result is a Struct because Geometry values always contain WGS 84 longitude and latitude.",
        ),
        FromWebMercator => (
            "geometry_from_web_mercator",
            "fromWebMercator",
            "Web Mercator to Geometry",
            "Converts an EPSG:3857 GeoJSON-like Struct in meters to a validated WGS 84 Geometry. All projected positions must be inside the Web Mercator world bounds.",
        ),
        WrapLongitude => (
            "geometry_wrap_longitude",
            "wrapLongitude",
            "Wrap Geometry Longitude",
            "Converts a GeoJSON-like Point, MultiPoint, or point-only GeometryCollection Struct with arbitrary finite longitudes into Geometry. Longitudes are wrapped to the half-open interval [-180, 180), while latitudes must already be within [-90, 90]. Connected geometries are rejected because wrapping individual vertices would tear them across the antimeridian.",
        ),
        GeodesicCircle => (
            "geometry_geodesic_circle",
            "geodesicCircle",
            "Geodesic Circle",
            "Approximates a WGS 84 geodesic circle around a Point using 3 through 1024 segments. Antimeridian crossings are split into valid polygon parts, and circles that contain a pole are rejected.",
        ),
    };

    let mut node = Node::new(id, title, description, "Web/Geo/Geometry");
    node.set_flowscript_name("geometry", alias);
    node.add_icon("/flow/icons/map.svg");
    match operation {
        ToWebMercator => {
            geometry_input(&mut node, "geometry", None);
            if let Some(pin) = node.pins.values_mut().find(|pin| pin.name == "geometry") {
                pin.description =
                    "Validated WGS 84 geometry with latitude within +/-85.0511 degrees".to_string();
            }
            node.add_output_pin(
                "projected",
                "Projected",
                "GeoJSON-like EPSG:3857 geometry with coordinates in meters",
                VariableType::Struct,
            );
        }
        FromWebMercator => {
            node.add_input_pin(
                "projected",
                "projected",
                "GeoJSON-like Struct marked with crs EPSG:3857",
                VariableType::Struct,
            );
            geometry_output(&mut node, None);
        }
        WrapLongitude => {
            node.add_input_pin(
                "geometry",
                "geometry",
                "GeoJSON-like Point, MultiPoint, or point-only GeometryCollection Struct with finite longitudes and WGS 84 latitudes",
                VariableType::Struct,
            );
            geometry_output(&mut node, None);
            node.set_receiver("");
        }
        GeodesicCircle => {
            geometry_input(&mut node, "origin", Some(GeometryKind::Point));
            node.add_input_pin(
                "radius",
                "radius",
                "Positive radius in meters",
                VariableType::Float,
            );
            node.add_input_pin(
                "segments",
                "segments",
                "Number of polygon segments from 3 through 1024",
                VariableType::Integer,
            );
            geometry_output(&mut node, None);
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
    operation: Operation,
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
pub struct GeometryToWebMercatorNode;
implement_node!(GeometryToWebMercatorNode, Operation::ToWebMercator);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryFromWebMercatorNode;
implement_node!(GeometryFromWebMercatorNode, Operation::FromWebMercator);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryWrapLongitudeNode;
implement_node!(GeometryWrapLongitudeNode, Operation::WrapLongitude);

#[crate::register_node]
#[derive(Default)]
pub struct GeometryGeodesicCircleNode;
implement_node!(GeometryGeodesicCircleNode, Operation::GeodesicCircle);

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn wrap_longitude_is_a_struct_to_geometry_boundary() {
        let node = definition(Operation::WrapLongitude);
        let input = node
            .pins
            .values()
            .find(|pin| pin.name == "geometry")
            .expect("geometry input");
        let output = node
            .pins
            .values()
            .find(|pin| pin.name == "geometry_out")
            .expect("geometry output");

        assert_eq!(input.data_type, VariableType::Struct);
        assert_eq!(output.data_type, VariableType::Geometry);
        assert_eq!(node.receiver.as_deref(), Some(""));
        assert_eq!(node.flowscript_receiver(), None);
    }
}

#[cfg(feature = "execute")]
mod runtime {
    use super::super::check_topology_validation;
    use super::Operation;
    use flow_like_geometry::{from_geo, to_geo};
    use flow_like_types::{
        Result, Value, anyhow, bail,
        geometry::{
            GeometryKind, MAX_GEOMETRY_BYTES, MAX_GEOMETRY_DEPTH, MAX_GEOMETRY_POSITIONS,
            canonicalize_geometry,
        },
        json::{Map, json},
    };
    use geo::{
        BooleanOps, Coord, Destination, Distance, Geodesic, Geometry, MapCoords, MultiPolygon,
        Point, Polygon, Rect, Translate,
    };
    use std::f64::consts::PI;

    const WEB_MERCATOR_CRS: &str = "EPSG:3857";
    const WGS84_CRS: &str = "EPSG:4326";
    const EARTH_RADIUS_METERS: f64 = 6_378_137.0;
    const MAX_MERCATOR_LATITUDE: f64 = 85.051_128_779_806_6;
    const MAX_MERCATOR_COORDINATE: f64 = 20_037_508.342_789_244;
    const MAX_CIRCLE_RADIUS_METERS: f64 = 10_000_000.0;
    // Polygon validation is quadratic in ring segments. This leaves enough
    // room for the extra clipping edges created by an antimeridian split.
    const MAX_CIRCLE_SEGMENTS: usize = 1024;

    macro_rules! ensure {
        ($condition:expr, $($message:tt)*) => {
            if !$condition { bail!($($message)*); }
        };
    }

    #[derive(Clone, Copy)]
    enum Transform {
        ForwardMercator,
        InverseMercator,
        WrapLongitude,
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

    fn integer(inputs: &Value, name: &str) -> Result<usize> {
        let value = input(inputs, name)?
            .as_i64()
            .ok_or_else(|| anyhow!("{name} must be an integer"))?;
        ensure!(value >= 0, "{name} must be nonnegative");
        usize::try_from(value).map_err(Into::into)
    }

    fn finite_position(value: &Value) -> Result<(f64, f64)> {
        let position = value
            .as_array()
            .filter(|position| position.len() == 2)
            .ok_or_else(|| anyhow!("A geometry position requires exactly two coordinates"))?;
        let x = position[0]
            .as_f64()
            .filter(|coordinate| coordinate.is_finite())
            .ok_or_else(|| anyhow!("Geometry x coordinate must be finite"))?;
        let y = position[1]
            .as_f64()
            .filter(|coordinate| coordinate.is_finite())
            .ok_or_else(|| anyhow!("Geometry y coordinate must be finite"))?;
        Ok((x, y))
    }

    fn transform_position(
        value: &Value,
        transform: Transform,
        positions: &mut usize,
    ) -> Result<Value> {
        *positions += 1;
        ensure!(
            *positions <= MAX_GEOMETRY_POSITIONS,
            "Geometry exceeds the position limit"
        );
        let (x, y) = finite_position(value)?;
        let output = match transform {
            Transform::ForwardMercator => {
                ensure!(
                    (-180.0..=180.0).contains(&x)
                        && (-MAX_MERCATOR_LATITUDE..=MAX_MERCATOR_LATITUDE).contains(&y),
                    "Web Mercator supports longitude [-180,180] and latitude [{},{}]",
                    -MAX_MERCATOR_LATITUDE,
                    MAX_MERCATOR_LATITUDE
                );
                let projected_x = EARTH_RADIUS_METERS * x.to_radians();
                let projected_y =
                    EARTH_RADIUS_METERS * (PI / 4.0 + y.to_radians() / 2.0).tan().ln();
                (
                    projected_x.clamp(-MAX_MERCATOR_COORDINATE, MAX_MERCATOR_COORDINATE),
                    projected_y.clamp(-MAX_MERCATOR_COORDINATE, MAX_MERCATOR_COORDINATE),
                )
            }
            Transform::InverseMercator => {
                ensure!(
                    x.abs() <= MAX_MERCATOR_COORDINATE && y.abs() <= MAX_MERCATOR_COORDINATE,
                    "EPSG:3857 coordinates must be inside [{0},{1}] meters on both axes",
                    -MAX_MERCATOR_COORDINATE,
                    MAX_MERCATOR_COORDINATE
                );
                let longitude = (x / EARTH_RADIUS_METERS).to_degrees().clamp(-180.0, 180.0);
                let latitude = (2.0 * (y / EARTH_RADIUS_METERS).exp().atan() - PI / 2.0)
                    .to_degrees()
                    .clamp(-MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
                (longitude, latitude)
            }
            Transform::WrapLongitude => {
                let modulo = x.rem_euclid(360.0);
                let longitude = if modulo >= 180.0 {
                    modulo - 360.0
                } else {
                    modulo
                };
                (longitude, y)
            }
        };
        Ok(json!([output.0, output.1]))
    }

    fn transform_nested(
        value: &Value,
        nesting: usize,
        transform: Transform,
        positions: &mut usize,
    ) -> Result<Value> {
        if nesting == 0 {
            return transform_position(value, transform, positions);
        }
        let values = value
            .as_array()
            .ok_or_else(|| anyhow!("Geometry coordinates have invalid array nesting"))?;
        Ok(Value::Array(
            values
                .iter()
                .map(|value| transform_nested(value, nesting - 1, transform, positions))
                .collect::<Result<Vec<_>>>()?,
        ))
    }

    fn coordinate_nesting(kind: GeometryKind) -> Option<usize> {
        match kind {
            GeometryKind::Point => Some(0),
            GeometryKind::LineString | GeometryKind::MultiPoint => Some(1),
            GeometryKind::Polygon | GeometryKind::MultiLineString => Some(2),
            GeometryKind::MultiPolygon => Some(3),
            GeometryKind::GeometryCollection => None,
        }
    }

    fn transform_geometry(
        value: &Value,
        transform: Transform,
        depth: usize,
        positions: &mut usize,
    ) -> Result<Value> {
        ensure!(
            depth <= MAX_GEOMETRY_DEPTH,
            "Geometry exceeds the depth limit"
        );
        let mut object: Map<String, Value> = value
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow!("Geometry must be an object"))?;
        let kind: GeometryKind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Geometry requires a type"))?
            .parse()?;
        object.remove("bbox");
        if let Some(crs) = object.remove("crs") {
            let expected = match transform {
                Transform::InverseMercator => WEB_MERCATOR_CRS,
                Transform::ForwardMercator | Transform::WrapLongitude => WGS84_CRS,
            };
            ensure!(
                crs.as_str() == Some(expected),
                "Geometry crs must be {expected}"
            );
        }

        if let Some(nesting) = coordinate_nesting(kind) {
            let coordinates = object
                .get("coordinates")
                .ok_or_else(|| anyhow!("{kind} requires coordinates"))?;
            object.insert(
                "coordinates".into(),
                transform_nested(coordinates, nesting, transform, positions)?,
            );
        } else {
            let geometries = object
                .get("geometries")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("GeometryCollection requires geometries"))?;
            object.insert(
                "geometries".into(),
                Value::Array(
                    geometries
                        .iter()
                        .map(|geometry| {
                            transform_geometry(geometry, transform, depth + 1, positions)
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            );
        }
        Ok(Value::Object(object))
    }

    fn checked_size(value: &Value) -> Result<()> {
        ensure!(
            flow_like_types::json::to_vec(value)?.len() <= MAX_GEOMETRY_BYTES,
            "Geometry exceeds the byte limit"
        );
        Ok(())
    }

    fn checked_geometry(value: &Value) -> Result<Value> {
        let value = canonicalize_geometry(value, None)?;
        check_topology_validation(&to_geo(&value)?, "Invalid geometry topology")?;
        Ok(value)
    }

    fn remove_wgs84_crs(value: &Value, depth: usize) -> Result<Value> {
        ensure!(
            depth <= MAX_GEOMETRY_DEPTH,
            "Geometry exceeds the depth limit"
        );
        let mut object = value
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow!("Geometry must be an object"))?;
        let kind: GeometryKind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Geometry requires a type"))?
            .parse()?;
        if let Some(crs) = object.remove("crs") {
            ensure!(
                crs.as_str() == Some(WGS84_CRS),
                "Geometry crs must be {WGS84_CRS}"
            );
        }
        if kind == GeometryKind::GeometryCollection {
            let geometries = object
                .get("geometries")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("GeometryCollection requires geometries"))?;
            object.insert(
                "geometries".into(),
                Value::Array(
                    geometries
                        .iter()
                        .map(|geometry| remove_wgs84_crs(geometry, depth + 1))
                        .collect::<Result<Vec<_>>>()?,
                ),
            );
        }
        Ok(Value::Object(object))
    }

    fn to_web_mercator(value: &Value) -> Result<Value> {
        checked_size(value)?;
        let canonical = checked_geometry(&remove_wgs84_crs(value, 0)?)?;
        let mut positions = 0;
        let mut projected =
            transform_geometry(&canonical, Transform::ForwardMercator, 0, &mut positions)?;
        projected
            .as_object_mut()
            .expect("transformed geometry object")
            .insert("crs".into(), json!(WEB_MERCATOR_CRS));
        checked_size(&projected)?;
        Ok(projected)
    }

    fn from_web_mercator(value: &Value) -> Result<Value> {
        checked_size(value)?;
        ensure!(
            value.get("crs").and_then(Value::as_str) == Some(WEB_MERCATOR_CRS),
            "Projected geometry must declare crs EPSG:3857"
        );
        let mut positions = 0;
        let geometry = transform_geometry(value, Transform::InverseMercator, 0, &mut positions)?;
        checked_geometry(&geometry)
    }

    /// Wrapping is only sound where positions are independent. Shifting a single
    /// vertex of a connected geometry from +180 to -180 turns the implied edge
    /// into a 359 degree span under the planar convention this catalog uses.
    fn ensure_wrappable(value: &Value, depth: usize) -> Result<()> {
        ensure!(
            depth <= MAX_GEOMETRY_DEPTH,
            "Geometry exceeds the depth limit"
        );
        let kind: GeometryKind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Geometry requires a type"))?
            .parse()?;
        match kind {
            GeometryKind::Point | GeometryKind::MultiPoint => Ok(()),
            GeometryKind::GeometryCollection => value
                .get("geometries")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("GeometryCollection requires geometries"))?
                .iter()
                .try_for_each(|geometry| ensure_wrappable(geometry, depth + 1)),
            kind => bail!(
                "Wrapping longitude per position would tear a {kind} across the antimeridian; wrap a Point or MultiPoint, or split the geometry at the antimeridian first"
            ),
        }
    }

    fn wrap_longitude(value: &Value) -> Result<Value> {
        checked_size(value)?;
        ensure_wrappable(value, 0)?;
        let mut positions = 0;
        let wrapped = transform_geometry(value, Transform::WrapLongitude, 0, &mut positions)?;
        checked_geometry(&wrapped)
    }

    fn circle_origin(value: &Value) -> Result<Point<f64>> {
        match to_geo(&checked_geometry(value)?)? {
            Geometry::Point(point) => Ok(point),
            _ => bail!("A geodesic circle requires a Point origin"),
        }
    }

    /// Samples the circle in continuous longitude space so consecutive vertices
    /// never jump across the antimeridian.
    fn circle_ring(origin: Point<f64>, radius: f64, segments: usize) -> Vec<Coord<f64>> {
        let mut coords: Vec<Coord<f64>> = Vec::with_capacity(segments + 1);
        for index in 0..segments {
            let bearing = 360.0 * index as f64 / segments as f64;
            let vertex = Geodesic.destination(origin, bearing, radius);
            let longitude = match coords.last() {
                Some(previous) => vertex.x() + 360.0 * ((previous.x - vertex.x()) / 360.0).round(),
                None => vertex.x(),
            };
            coords.push(Coord {
                x: longitude,
                y: vertex.y(),
            });
        }
        coords.push(coords[0]);
        coords
    }

    fn world_window(offset: f64) -> Polygon<f64> {
        Rect::new(
            Coord {
                x: -180.0 + offset,
                y: -90.0,
            },
            Coord {
                x: 180.0 + offset,
                y: 90.0,
            },
        )
        .to_polygon()
    }

    fn split_at_antimeridian(circle: &Polygon<f64>) -> Vec<Polygon<f64>> {
        let mut parts = Vec::new();
        for offset in [-360.0, 0.0, 360.0] {
            for part in circle.intersection(&world_window(offset)) {
                parts.push(part.translate(-offset, 0.0).map_coords(|coord| Coord {
                    x: coord.x.clamp(-180.0, 180.0),
                    y: coord.y.clamp(-90.0, 90.0),
                }));
            }
        }
        parts
    }

    fn geodesic_circle(inputs: &Value) -> Result<Value> {
        let origin = circle_origin(input(inputs, "origin")?)?;
        let radius = number(inputs, "radius")?;
        ensure!(
            radius > 0.0 && radius <= MAX_CIRCLE_RADIUS_METERS,
            "Circle radius must be greater than zero and at most {MAX_CIRCLE_RADIUS_METERS} meters, received {radius}"
        );
        let segments = integer(inputs, "segments")?;
        ensure!(
            (3..=MAX_CIRCLE_SEGMENTS).contains(&segments),
            "Circle segments must be between 3 and {MAX_CIRCLE_SEGMENTS}, received {segments}"
        );
        for (latitude, pole) in [(90.0, "north"), (-90.0, "south")] {
            let pole_distance = Geodesic.distance(origin, Point::new(origin.x(), latitude));
            ensure!(
                (0.0..pole_distance).contains(&radius),
                "A circle containing the {pole} pole has no longitude/latitude polygon representation"
            );
        }

        let coords = circle_ring(origin, radius, segments);
        let crosses_antimeridian = coords
            .iter()
            .any(|coord| !(-180.0..=180.0).contains(&coord.x));
        let circle = Polygon::new(coords.into(), Vec::new());
        let mut parts = if crosses_antimeridian {
            split_at_antimeridian(&circle)
        } else {
            vec![circle]
        };
        ensure!(
            !parts.is_empty(),
            "The geodesic circle produced no polygon inside the world bounds"
        );

        let geometry = if parts.len() == 1 {
            Geometry::Polygon(parts.remove(0))
        } else {
            Geometry::MultiPolygon(MultiPolygon::new(parts))
        };
        check_topology_validation(&geometry, "The geodesic circle is not a valid polygon")?;
        let encoded = canonicalize_geometry(&from_geo(&geometry)?, None)?;
        checked_size(&encoded)?;
        Ok(encoded)
    }

    pub(super) fn execute(
        operation: Operation,
        inputs: &Value,
    ) -> Result<Vec<(&'static str, Value)>> {
        use Operation::*;
        Ok(match operation {
            ToWebMercator => vec![("projected", to_web_mercator(input(inputs, "geometry")?)?)],
            FromWebMercator => vec![(
                "geometry_out",
                from_web_mercator(input(inputs, "projected")?)?,
            )],
            WrapLongitude => vec![("geometry_out", wrap_longitude(input(inputs, "geometry")?)?)],
            GeodesicCircle => vec![("geometry_out", geodesic_circle(inputs)?)],
        })
    }

    #[cfg(all(test, feature = "execute"))]
    mod tests {
        use super::*;

        fn output(operation: Operation, inputs: Value, name: &str) -> Value {
            execute(operation, &inputs)
                .unwrap()
                .into_iter()
                .find(|(output_name, _)| *output_name == name)
                .unwrap()
                .1
        }

        fn point(x: f64, y: f64) -> Value {
            json!({"type": "Point", "coordinates": [x, y]})
        }

        fn circle(origin: Value, radius: f64, segments: i64) -> Result<Value> {
            execute(
                Operation::GeodesicCircle,
                &json!({"origin": origin, "radius": radius, "segments": segments}),
            )
            .map(|outputs| {
                outputs
                    .into_iter()
                    .find(|(name, _)| *name == "geometry_out")
                    .expect("geometry output")
                    .1
            })
        }

        fn rings(value: &Value) -> Vec<Vec<(f64, f64)>> {
            let polygons = match value["type"].as_str().expect("geometry type") {
                "Polygon" => vec![value["coordinates"].clone()],
                "MultiPolygon" => value["coordinates"]
                    .as_array()
                    .expect("multipolygon parts")
                    .clone(),
                other => panic!("unexpected geometry {other}"),
            };
            polygons
                .iter()
                .map(|polygon| {
                    polygon[0]
                        .as_array()
                        .expect("exterior ring")
                        .iter()
                        .map(|position| {
                            (
                                position[0].as_f64().expect("longitude"),
                                position[1].as_f64().expect("latitude"),
                            )
                        })
                        .collect()
                })
                .collect()
        }

        #[test]
        fn web_mercator_round_trips_within_projection_tolerance() {
            let projected = output(
                Operation::ToWebMercator,
                json!({"geometry": point(13.404954, 52.520008)}),
                "projected",
            );
            assert_eq!(projected["crs"], json!("EPSG:3857"));
            assert!((projected["coordinates"][0].as_f64().unwrap() - 1_492_232.65).abs() < 0.1);
            assert!((projected["coordinates"][1].as_f64().unwrap() - 6_894_701.26).abs() < 0.1);

            let restored = output(
                Operation::FromWebMercator,
                json!({"projected": projected}),
                "geometry_out",
            );
            assert_eq!(restored["type"], "Point");
            assert!((restored["coordinates"][0].as_f64().unwrap() - 13.404954).abs() < 1e-9);
            assert!((restored["coordinates"][1].as_f64().unwrap() - 52.520008).abs() < 1e-9);

            assert!(
                to_web_mercator(&json!({
                    "type": "Point",
                    "coordinates": [13.404954, 52.520008],
                    "crs": "EPSG:4326"
                }))
                .is_ok()
            );
            assert!(
                to_web_mercator(&json!({
                    "type": "Point",
                    "coordinates": [13.404954, 52.520008],
                    "crs": "EPSG:3857"
                }))
                .is_err()
            );
        }

        #[test]
        fn from_web_mercator_requires_the_projected_crs_and_world_bounds() {
            assert!(
                from_web_mercator(&json!({"type": "Point", "coordinates": [0.0, 0.0]})).is_err()
            );
            assert!(
                from_web_mercator(
                    &json!({"type": "Point", "coordinates": [3.0e7, 0.0], "crs": "EPSG:3857"})
                )
                .is_err()
            );
            assert!(
                from_web_mercator(&json!({
                    "type": "Polygon",
                    "coordinates": [[
                        [0.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0],
                        [1000.0, 0.0], [0.0, 0.0]
                    ]],
                    "crs": "EPSG:3857"
                }))
                .is_err()
            );
        }

        #[test]
        fn wrap_longitude_maps_into_the_half_open_interval() {
            let wrapped = output(
                Operation::WrapLongitude,
                json!({"geometry": {
                    "type": "MultiPoint",
                    "coordinates": [
                        [180.0, 10.0], [-180.0, 10.0], [-0.5, 10.0],
                        [540.0, 10.0], [-541.0, 10.0]
                    ]
                }}),
                "geometry_out",
            );
            assert_eq!(
                wrapped["coordinates"],
                json!([
                    [-180.0, 10.0],
                    [-180.0, 10.0],
                    [-0.5, 10.0],
                    [-180.0, 10.0],
                    [179.0, 10.0]
                ])
            );

            let very_large = wrap_longitude(&point(f64::MAX, 10.0)).expect("finite longitude");
            let longitude = very_large["coordinates"][0]
                .as_f64()
                .expect("wrapped longitude");
            assert!((-180.0..180.0).contains(&longitude));
        }

        #[test]
        fn wrap_longitude_enforces_wgs84_latitude_and_crs() {
            assert!(wrap_longitude(&point(540.0, 90.0)).is_ok());
            assert!(wrap_longitude(&point(540.0, 90.000_001)).is_err());
            assert!(
                wrap_longitude(&json!({
                    "type": "Point",
                    "coordinates": [540.0, 10.0],
                    "crs": "EPSG:4326"
                }))
                .is_ok()
            );
            assert!(
                wrap_longitude(&json!({
                    "type": "Point",
                    "coordinates": [540.0, 10.0],
                    "crs": "EPSG:3857"
                }))
                .is_err()
            );
        }

        #[test]
        fn wrap_longitude_refuses_to_tear_connected_geometries() {
            let line = json!({"type": "LineString", "coordinates": [[179.0, 0.0], [180.0, 0.0]]});
            let error = wrap_longitude(&line).expect_err("connected geometry must be rejected");
            assert!(error.to_string().contains("tear"), "{error}");

            let fiji = json!({"type": "Polygon", "coordinates": [[
                [179.0, -16.0], [180.0, -16.0], [180.0, -17.0], [179.0, -17.0], [179.0, -16.0]
            ]]});
            assert!(wrap_longitude(&fiji).is_err());

            let collection = json!({"type": "GeometryCollection", "geometries": [
                {"type": "Point", "coordinates": [180.0, 0.0]},
                line
            ]});
            assert!(wrap_longitude(&collection).is_err());
        }

        #[test]
        fn wrap_longitude_still_accepts_independent_positions() {
            let collection = json!({"type": "GeometryCollection", "geometries": [
                {"type": "Point", "coordinates": [180.0, 0.0]},
                {"type": "MultiPoint", "coordinates": [[180.0, 1.0], [-0.5, 2.0]]}
            ]});
            let wrapped = wrap_longitude(&collection).expect("point collection wraps");
            assert_eq!(
                wrapped["geometries"][0]["coordinates"],
                json!([-180.0, 0.0])
            );
            assert_eq!(
                wrapped["geometries"][1]["coordinates"],
                json!([[-180.0, 1.0], [-0.5, 2.0]])
            );
        }

        #[test]
        fn to_web_mercator_rejects_positions_beyond_the_projection_bound() {
            let circle = circle(point(0.0, 88.0), 100_000.0, 64).expect("polar circle");
            let error = to_web_mercator(&circle).expect_err("polar circle has no EPSG:3857 form");
            assert!(error.to_string().contains("latitude"), "{error}");
        }

        #[test]
        fn equatorial_circle_closes_and_keeps_the_requested_radius() {
            let origin = Point::new(0.0, 0.0);
            let value = circle(point(0.0, 0.0), 100_000.0, 64).expect("circle");
            assert_eq!(value["type"], "Polygon");
            let ring = rings(&value).remove(0);
            assert_eq!(ring.len(), 65);
            assert_eq!(ring[0], ring[64]);
            for (longitude, latitude) in ring.iter().take(64) {
                let distance = Geodesic.distance(origin, Point::new(*longitude, *latitude));
                assert!(
                    (distance - 100_000.0).abs() < 1.0,
                    "vertex distance {distance} differs from the requested radius"
                );
            }
        }

        #[test]
        fn antimeridian_circle_splits_into_parts_inside_the_world_bounds() {
            let value = circle(point(180.0, 0.0), 200_000.0, 64).expect("circle");
            assert_eq!(value["type"], "MultiPolygon");
            let parts = rings(&value);
            assert_eq!(parts.len(), 2);
            for ring in &parts {
                assert_eq!(ring.first(), ring.last());
                for (longitude, latitude) in ring {
                    assert!((-180.0..=180.0).contains(longitude));
                    assert!((-90.0..=90.0).contains(latitude));
                }
            }
            assert!(parts.iter().any(|ring| ring.iter().all(|(x, _)| *x <= 0.0)));
            assert!(parts.iter().any(|ring| ring.iter().all(|(x, _)| *x >= 0.0)));
        }

        #[test]
        fn pole_containing_circles_are_rejected() {
            assert!(circle(point(0.0, 89.9), 100_000.0, 64).is_err());
            assert!(circle(point(0.0, -89.9), 100_000.0, 64).is_err());
            assert!(circle(point(0.0, 88.0), 100_000.0, 64).is_ok());
        }

        #[test]
        fn circle_inputs_are_bounded() {
            assert!(circle(point(0.0, 0.0), 0.0, 64).is_err());
            assert!(circle(point(0.0, 0.0), -1.0, 64).is_err());
            assert!(circle(point(0.0, 0.0), MAX_CIRCLE_RADIUS_METERS + 1.0, 64).is_err());
            assert!(circle(point(0.0, 0.0), 1_000.0, 2).is_err());
            assert!(circle(point(0.0, 0.0), 1_000.0, MAX_CIRCLE_SEGMENTS as i64 + 1).is_err());
            assert!(circle(point(0.0, 0.0), 1_000.0, 3).is_ok());
            assert!(circle(point(180.0, 0.0), 1_000.0, MAX_CIRCLE_SEGMENTS as i64).is_ok());
            assert!(
                circle(
                    json!({"type": "LineString", "coordinates": [[0.0, 0.0], [1.0, 1.0]]}),
                    1_000.0,
                    8
                )
                .is_err()
            );
        }
    }
}
