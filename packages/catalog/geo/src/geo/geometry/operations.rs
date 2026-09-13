use super::nodes::Operation;
use super::{check_topology_validation, ensure_geometry_pair_budget};
use flow_like_geometry::{from_geo, from_wkb, from_wkt, to_geo, to_wkb, to_wkt};
use flow_like_types::{
    Result, Value, anyhow, bail,
    geometry::{GeometryKind, canonicalize_geometry, validate_geometry},
    json::json,
};
use geo::{
    Area, BooleanOps, BoundingRect, Centroid, Contains, ConvexHull, Coord, CoordsIter, Distance,
    Euclidean, Geodesic, GeodesicArea, Geometry, Intersects, Length, LineString, MapCoords,
    MultiPolygon, Polygon, Rect, Simplify,
};

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
    let number = input(inputs, name)?
        .as_f64()
        .ok_or_else(|| anyhow!("{name} must be a finite number"))?;
    ensure!(number.is_finite(), "{name} must be finite");
    Ok(number)
}
fn text<'a>(inputs: &'a Value, name: &str) -> Result<&'a str> {
    input(inputs, name)?
        .as_str()
        .ok_or_else(|| anyhow!("{name} must be a string"))
}
fn checked_geometry(value: &Value) -> Result<Geometry<f64>> {
    let geometry = to_geo(&canonicalize_geometry(value, None)?)?;
    check_topology_validation(&geometry, "Invalid geometry for spatial operation")?;
    Ok(geometry)
}
fn encoded_geometry(geometry: Geometry<f64>) -> Result<Value> {
    check_topology_validation(&geometry, "Spatial operation produced invalid geometry")?;
    Ok(canonicalize_geometry(&from_geo(&geometry)?, None)?)
}
fn finite_number(value: f64) -> Result<Value> {
    ensure!(
        value.is_finite(),
        "Geometry operation produced a non-finite measurement"
    );
    Ok(json!(value))
}

/// Legacy coordinates are latitude/longitude objects; GeoJSON positions are longitude/latitude.
fn legacy_position(value: &Value) -> Result<Value> {
    let position = json!([number(value, "longitude")?, number(value, "latitude")?]);
    validate_geometry(
        &json!({"type":"Point", "coordinates":position}),
        Some(GeometryKind::Point),
    )?;
    Ok(position)
}
fn legacy_positions(value: &Value) -> Result<Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| anyhow!("Expected an array of legacy GeoCoordinate objects"))?
        .iter()
        .map(legacy_position)
        .collect()
}
fn closed_legacy_ring(value: &Value) -> Result<Vec<Value>> {
    let mut ring = legacy_positions(value)?;
    ensure!(
        ring.len() >= 3,
        "A polygon boundary requires at least three vertices"
    );
    if ring.first() != ring.last() {
        ring.push(ring[0].clone());
    }
    Ok(ring)
}
fn legacy_polygon(value: &Value) -> Result<Value> {
    let mut rings = vec![Value::Array(closed_legacy_ring(input(value, "exterior")?)?)];
    if let Some(interiors) = value.get("interiors") {
        for ring in interiors
            .as_array()
            .ok_or_else(|| anyhow!("Polygon interiors must be an array"))?
        {
            rings.push(Value::Array(closed_legacy_ring(ring)?));
        }
    }
    Ok(Value::Array(rings))
}

fn split_legacy_h3_polygon(polygon: Polygon<f64>) -> Result<Vec<Polygon<f64>>> {
    let crosses_antimeridian = polygon
        .exterior()
        .0
        .windows(2)
        .any(|edge| (edge[0].x - edge[1].x).abs() > 180.0);
    if !crosses_antimeridian {
        return Ok(vec![polygon]);
    }

    let shifted = polygon.map_coords(|coordinate| Coord {
        x: if coordinate.x < 0.0 {
            coordinate.x + 360.0
        } else {
            coordinate.x
        },
        y: coordinate.y,
    });
    check_topology_validation(&shifted, "Invalid transmeridian legacy H3 polygon")?;
    let western_window =
        Rect::new(Coord { x: 180.0, y: -90.0 }, Coord { x: 360.0, y: 90.0 }).to_polygon();
    let eastern_window =
        Rect::new(Coord { x: 0.0, y: -90.0 }, Coord { x: 180.0, y: 90.0 }).to_polygon();

    let mut parts = shifted
        .intersection(&western_window)
        .0
        .into_iter()
        .map(|part| {
            part.map_coords(|coordinate| Coord {
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
                part.map_coords(|coordinate| Coord {
                    x: coordinate.x.clamp(0.0, 180.0),
                    y: coordinate.y.clamp(-90.0, 90.0),
                })
            }),
    );
    ensure!(
        !parts.is_empty(),
        "Legacy H3 polygon produced no parts inside WGS 84 bounds"
    );
    Ok(parts)
}

/// Legacy H3 boundaries are unclosed latitude/longitude vectors. h3o emits
/// transmeridian cells with vertices on both sides of +/-180, so split those
/// cells before returning a planar GeoJSON geometry.
fn legacy_boundary_geometry(value: &Value) -> Result<Geometry<f64>> {
    let ring = closed_legacy_ring(value)?;
    let polygon = Polygon::new(
        ring.iter()
            .map(|position| Coord {
                x: position[0].as_f64().unwrap(),
                y: position[1].as_f64().unwrap(),
            })
            .collect::<Vec<_>>()
            .into(),
        Vec::new(),
    );
    check_topology_validation(&polygon, "Invalid legacy H3 boundary")?;
    let mut parts = split_legacy_h3_polygon(polygon)?;
    Ok(if parts.len() == 1 {
        Geometry::Polygon(parts.remove(0))
    } else {
        Geometry::MultiPolygon(MultiPolygon::new(parts))
    })
}

fn legacy_h3_multipolygon(value: &Value) -> Result<Geometry<f64>> {
    let polygons = value
        .as_array()
        .ok_or_else(|| anyhow!("Expected the legacy H3 polygon array"))?;
    let mut parts = Vec::new();
    for polygon in polygons {
        let value = json!({"type":"Polygon", "coordinates":legacy_polygon(polygon)?});
        let Geometry::Polygon(polygon) =
            to_geo(&canonicalize_geometry(&value, Some(GeometryKind::Polygon))?)?
        else {
            unreachable!("the constructed legacy value is a Polygon");
        };
        parts.extend(split_legacy_h3_polygon(polygon)?);
    }
    Ok(Geometry::MultiPolygon(MultiPolygon::new(parts)))
}
fn coordinate_object(position: &Value) -> Value {
    json!({"latitude":position[1], "longitude":position[0]})
}
fn line_length(line: &LineString<f64>, geodesic: bool) -> f64 {
    if geodesic {
        Geodesic.length(line)
    } else {
        Euclidean.length(line)
    }
}
fn geometry_length(geometry: &Geometry<f64>, geodesic: bool) -> Result<f64> {
    Ok(match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => 0.0,
        Geometry::LineString(line) => line_length(line, geodesic),
        Geometry::MultiLineString(lines) => {
            lines.0.iter().map(|line| line_length(line, geodesic)).sum()
        }
        Geometry::Polygon(polygon) => std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .map(|line| line_length(line, geodesic))
            .sum(),
        Geometry::MultiPolygon(polygons) => polygons
            .0
            .iter()
            .map(|polygon| geometry_length(&Geometry::Polygon(polygon.clone()), geodesic))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum(),
        Geometry::GeometryCollection(collection) => collection
            .0
            .iter()
            .map(|geometry| geometry_length(geometry, geodesic))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum(),
        _ => bail!("Unsupported geometry for length"),
    })
}
fn geometry_area(geometry: &Geometry<f64>, geodesic: bool) -> Result<f64> {
    Ok(match geometry {
        Geometry::Polygon(polygon) => {
            if geodesic {
                polygon.geodesic_area_signed().abs()
            } else {
                polygon.unsigned_area()
            }
        }
        Geometry::MultiPolygon(polygons) => polygons
            .0
            .iter()
            .map(|polygon| geometry_area(&Geometry::Polygon(polygon.clone()), geodesic))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum(),
        Geometry::GeometryCollection(collection) => collection
            .0
            .iter()
            .map(|geometry| geometry_area(geometry, geodesic))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .sum(),
        Geometry::Point(_)
        | Geometry::Line(_)
        | Geometry::LineString(_)
        | Geometry::MultiPoint(_)
        | Geometry::MultiLineString(_) => 0.0,
        Geometry::Rect(rect) => geometry_area(&Geometry::Polygon(rect.to_polygon()), geodesic)?,
        Geometry::Triangle(triangle) => {
            geometry_area(&Geometry::Polygon(triangle.to_polygon()), geodesic)?
        }
    })
}

fn natural_convex_hull(geometry: &Geometry<f64>) -> Result<Geometry<f64>> {
    let hull = geometry.convex_hull();
    let coordinates = &hull.exterior().0;
    match coordinates.as_slice() {
        [] => bail!("Empty geometry has no convex hull"),
        [point] | [point, _] => Ok(geo::Point(*point).into()),
        [start, end, _] => Ok(LineString(vec![*start, *end]).into()),
        _ => Ok(hull.into()),
    }
}
fn simplify(geometry: Geometry<f64>, tolerance: f64) -> Result<Geometry<f64>> {
    Ok(match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => geometry,
        Geometry::LineString(line) => line.simplify(tolerance).into(),
        Geometry::MultiLineString(lines) => lines.simplify(tolerance).into(),
        Geometry::Polygon(polygon) => polygon.simplify(tolerance).into(),
        Geometry::MultiPolygon(polygons) => polygons.simplify(tolerance).into(),
        Geometry::GeometryCollection(collection) => {
            Geometry::GeometryCollection(geo::GeometryCollection(
                collection
                    .0
                    .into_iter()
                    .map(|geometry| simplify(geometry, tolerance))
                    .collect::<Result<_>>()?,
            ))
        }
        _ => bail!("Unsupported geometry for simplify"),
    })
}

/// Execute the operation separately from graph I/O so units and legacy conversion are testable.
pub(super) fn execute(operation: Operation, inputs: &Value) -> Result<Vec<(&'static str, Value)>> {
    use Operation::*;
    let geometry_output = |value: Value| -> Result<Vec<(&'static str, Value)>> {
        Ok(vec![("geometry_out", canonicalize_geometry(&value, None)?)])
    };
    match operation {
        MakePoint => geometry_output(
            json!({"type":"Point", "coordinates":[number(inputs, "longitude")?, number(inputs, "latitude")?]}),
        ),
        Cast(kind) => {
            let value = canonicalize_geometry(input(inputs, "geometry")?, Some(kind))?;
            checked_geometry(&value)?;
            Ok(vec![("geometry_out", value)])
        }
        FromGeoJson => {
            let source = text(inputs, "text")?;
            ensure!(
                source.len() <= flow_like_types::geometry::MAX_GEOMETRY_BYTES,
                "GeoJSON exceeds geometry byte limit"
            );
            geometry_output(flow_like_types::json::from_str(source)?)
        }
        ToGeoJson => Ok(vec![(
            "text",
            json!(canonicalize_geometry(input(inputs, "geometry")?, None)?.to_string()),
        )]),
        FromWkt => geometry_output(from_wkt(text(inputs, "text")?)?),
        ToWkt => Ok(vec![("text", json!(to_wkt(input(inputs, "geometry")?)?))]),
        FromWkb => {
            let bytes: Vec<u8> =
                flow_like_types::json::from_value(input(inputs, "bytes")?.clone())?;
            geometry_output(from_wkb(&bytes)?)
        }
        ToWkb => Ok(vec![("bytes", json!(to_wkb(input(inputs, "geometry")?)?))]),
        FromCoordinate => geometry_output(
            json!({"type":"Point", "coordinates":legacy_position(input(inputs, "coordinate")?)?}),
        ),
        ToCoordinate => {
            let value = input(inputs, "geometry")?;
            validate_geometry(value, Some(GeometryKind::Point))?;
            Ok(vec![(
                "coordinate",
                coordinate_object(&value["coordinates"]),
            )])
        }
        FromBoundary => encoded_geometry(legacy_boundary_geometry(input(inputs, "boundary")?)?)
            .and_then(geometry_output),
        FromPolygons => encoded_geometry(legacy_h3_multipolygon(input(inputs, "polygons")?)?)
            .and_then(geometry_output),
        FromRoute => {
            let route = input(inputs, "route")?;
            let geometry = route.get("geometry").unwrap_or(route);
            let value = json!({"type":"LineString", "coordinates":legacy_positions(input(geometry, "points")?)?});
            let mut outputs = geometry_output(value)?;
            outputs.push(("route_out", route.clone()));
            Ok(outputs)
        }
        ToRoute => {
            let value = input(inputs, "geometry")?;
            validate_geometry(value, Some(GeometryKind::LineString))?;
            let points: Vec<Value> = value["coordinates"]
                .as_array()
                .unwrap()
                .iter()
                .map(coordinate_object)
                .collect();
            let mut route = input(inputs, "route")?.clone();
            let object = route
                .as_object_mut()
                .ok_or_else(|| anyhow!("Expected a legacy route object"))?;
            if object.contains_key("geometry") {
                object
                    .get_mut("geometry")
                    .unwrap()
                    .as_object_mut()
                    .ok_or_else(|| anyhow!("Route geometry must be an object"))?
                    .insert("points".into(), json!(points));
            } else {
                ensure!(
                    object.contains_key("points"),
                    "Expected RouteResult or RouteGeometry with points"
                );
                object.insert("points".into(), json!(points));
            }
            Ok(vec![("route_out", route)])
        }
        FromLocation => {
            let location = input(inputs, "location")?;
            let value = json!({"type":"Point", "coordinates":legacy_position(input(location, "coordinate")?)?});
            let mut outputs = geometry_output(value)?;
            outputs.push(("location_out", location.clone()));
            Ok(outputs)
        }
        Type => {
            let value = input(inputs, "geometry")?;
            validate_geometry(value, None)?;
            Ok(vec![("type", value["type"].clone())])
        }
        X | Y => {
            let value = input(inputs, "geometry")?;
            validate_geometry(value, Some(GeometryKind::Point))?;
            Ok(vec![(
                "value",
                value["coordinates"][if matches!(operation, X) { 0 } else { 1 }].clone(),
            )])
        }
        Bounds => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            let bounds = geometry
                .bounding_rect()
                .ok_or_else(|| anyhow!("Empty geometry has no bounds"))?;
            Ok(vec![
                ("min_longitude", finite_number(bounds.min().x)?),
                ("min_latitude", finite_number(bounds.min().y)?),
                ("max_longitude", finite_number(bounds.max().x)?),
                ("max_latitude", finite_number(bounds.max().y)?),
            ])
        }
        NumPoints => Ok(vec![(
            "count",
            json!(to_geo(input(inputs, "geometry")?)?.coords_iter().count()),
        )]),
        Contains | Intersects | Within => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            let operation_name = match operation {
                Contains => "Geometry contains predicate",
                Intersects => "Geometry intersects predicate",
                Within => "Geometry within predicate",
                _ => unreachable!(),
            };
            ensure_geometry_pair_budget(&a, &b, operation_name)?;
            Ok(vec![(
                "result",
                json!(match operation {
                    Contains => a.contains(&b),
                    Intersects => a.intersects(&b),
                    Within => b.contains(&a),
                    _ => unreachable!(),
                }),
            )])
        }
        Intersection => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            encoded_geometry(super::advanced::mixed_dimension_intersection(a, b)?)
                .and_then(geometry_output)
        }
        Centroid => {
            let centroid = checked_geometry(input(inputs, "geometry")?)?
                .centroid()
                .ok_or_else(|| anyhow!("Empty geometry has no centroid"))?;
            encoded_geometry(centroid.into()).and_then(geometry_output)
        }
        ConvexHull => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            encoded_geometry(natural_convex_hull(&geometry)?).and_then(geometry_output)
        }
        Simplify => {
            let tolerance = number(inputs, "tolerance")?;
            ensure!(
                tolerance >= 0.0,
                "Simplify tolerance must be nonnegative degrees"
            );
            let result = simplify(checked_geometry(input(inputs, "geometry")?)?, tolerance)?;
            encoded_geometry(result).and_then(geometry_output)
        }
        PlanarDistance | GeodesicDistance => {
            let a = checked_geometry(input(inputs, "a")?)?;
            let b = checked_geometry(input(inputs, "b")?)?;
            ensure!(
                a.coords_iter().next().is_some() && b.coords_iter().next().is_some(),
                "Empty geometries have no distance"
            );
            let distance = if matches!(operation, GeodesicDistance) {
                let (Geometry::Point(a), Geometry::Point(b)) = (a, b) else {
                    bail!("Geodesic distance currently requires two Points");
                };
                Geodesic.distance(a, b)
            } else {
                ensure_geometry_pair_budget(&a, &b, "Planar geometry distance")?;
                Euclidean.distance(&a, &b)
            };
            Ok(vec![("distance", finite_number(distance)?)])
        }
        PlanarLength | GeodesicLength => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            Ok(vec![(
                "length",
                finite_number(geometry_length(
                    &geometry,
                    matches!(operation, GeodesicLength),
                )?)?,
            )])
        }
        PlanarArea | GeodesicArea => {
            let geometry = checked_geometry(input(inputs, "geometry")?)?;
            let area = geometry_area(&geometry, matches!(operation, GeodesicArea))?;
            Ok(vec![("area", finite_number(area)?)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Validation;

    fn result(op: Operation, inputs: Value, output: &str) -> Value {
        execute(op, &inputs)
            .unwrap()
            .into_iter()
            .find(|(name, _)| *name == output)
            .unwrap()
            .1
    }
    fn point(longitude: f64, latitude: f64) -> Value {
        json!({"type":"Point", "coordinates":[longitude, latitude]})
    }
    fn square() -> Value {
        json!({"type":"Polygon", "coordinates":[[[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0],[0.0,0.0]]]})
    }

    #[test]
    fn geometry_constructors_and_casts_validate_coordinates_and_subtypes() {
        assert_eq!(
            result(
                Operation::MakePoint,
                json!({"longitude":13.405,"latitude":52.52}),
                "geometry_out"
            ),
            point(13.405, 52.52)
        );
        for values in [
            json!({"longitude":181,"latitude":0}),
            json!({"longitude":0,"latitude":91}),
            json!({"longitude":null,"latitude":0}),
        ] {
            assert!(execute(Operation::MakePoint, &values).is_err());
        }
        assert!(
            execute(
                Operation::Cast(GeometryKind::Polygon),
                &json!({"geometry":point(1.0,2.0)})
            )
            .is_err()
        );
        assert_eq!(
            result(
                Operation::Cast(GeometryKind::Point),
                json!({"geometry":point(1.0,2.0)}),
                "geometry_out"
            ),
            point(1.0, 2.0)
        );
        let bowtie = json!({"type":"Polygon","coordinates":[[[0,0],[2,2],[0,2],[2,0],[0,0]]]});
        assert!(
            execute(
                Operation::Cast(GeometryKind::Polygon),
                &json!({"geometry":bowtie})
            )
            .is_err()
        );
        assert!(
            execute(
                Operation::FromGeoJson,
                &json!({"text":"{\"type\":\"Feature\",\"geometry\":null}"})
            )
            .is_err()
        );
    }

    #[test]
    fn geometry_codecs_round_trip_and_geojson_retains_foreign_members() {
        let mut geometry = square();
        geometry["label"] = json!("keep in JSON");
        let text = result(Operation::ToGeoJson, json!({"geometry":geometry}), "text");
        assert_eq!(
            result(Operation::FromGeoJson, json!({"text":text}), "geometry_out")["label"],
            "keep in JSON"
        );
        let wkt = result(Operation::ToWkt, json!({"geometry":square()}), "text");
        assert_eq!(
            result(Operation::FromWkt, json!({"text":wkt}), "geometry_out"),
            square()
        );
        let bytes = result(Operation::ToWkb, json!({"geometry":square()}), "bytes");
        assert!(bytes.is_array());
        assert_eq!(
            result(Operation::FromWkb, json!({"bytes":bytes}), "geometry_out"),
            square()
        );
        assert!(execute(Operation::FromWkt, &json!({"text":"POINT Z (1 2 3)"})).is_err());
    }

    #[test]
    fn geometry_legacy_adapters_close_h3_rings_and_preserve_wrappers() {
        let coordinate = json!({"latitude":52.52, "longitude":13.405});
        let geometry = result(
            Operation::FromCoordinate,
            json!({"coordinate":coordinate}),
            "geometry_out",
        );
        assert_eq!(geometry, point(13.405, 52.52));
        assert_eq!(
            result(
                Operation::ToCoordinate,
                json!({"geometry":geometry}),
                "coordinate"
            ),
            coordinate
        );
        let boundary = json!([
            {"latitude":0,"longitude":0}, {"latitude":0,"longitude":4},
            {"latitude":4,"longitude":4}, {"latitude":4,"longitude":0}
        ]);
        let polygon = result(
            Operation::FromBoundary,
            json!({"boundary":boundary}),
            "geometry_out",
        );
        let ring = polygon["coordinates"][0].as_array().unwrap();
        assert_eq!(ring.first(), ring.last());
        assert_eq!(ring.len(), 5);
        let hole = json!([
            {"latitude":1,"longitude":1}, {"latitude":3,"longitude":1},
            {"latitude":3,"longitude":3}, {"latitude":1,"longitude":3}
        ]);
        let multi = result(
            Operation::FromPolygons,
            json!({"polygons":[{"exterior":boundary,"interiors":[hole]}]}),
            "geometry_out",
        );
        assert_eq!(multi["coordinates"][0].as_array().unwrap().len(), 2);
        assert_eq!(
            result(Operation::PlanarArea, json!({"geometry":multi}), "area"),
            12.0
        );
        let route = json!({"distance":123, "legs":[{"summary":"retain"}], "custom":true,
            "geometry":{"points":[coordinate,{"latitude":48.8566,"longitude":2.3522}], "precision":6}});
        let outputs = execute(Operation::FromRoute, &json!({"route":route})).unwrap();
        assert_eq!(outputs[0].1["coordinates"][0], json!([13.405, 52.52]));
        assert_eq!(outputs[1].1, route);
        let updated = result(
            Operation::ToRoute,
            json!({"route":route,"geometry":{"type":"LineString","coordinates":[[1,2],[3,4]]}}),
            "route_out",
        );
        assert_eq!(updated["legs"], route["legs"]);
        assert_eq!(updated["custom"], true);
        assert_eq!(updated["geometry"]["precision"], 6);
        assert_eq!(
            updated["geometry"]["points"][0],
            json!({"latitude":2,"longitude":1})
        );
        let location = json!({"coordinate":coordinate,"display_name":"Berlin","osm_id":42});
        let outputs = execute(Operation::FromLocation, &json!({"location":location})).unwrap();
        assert_eq!(outputs[1].1, location);
    }

    #[test]
    fn legacy_h3_boundary_splits_at_the_antimeridian() {
        use std::str::FromStr;

        let cell = h3o::CellIndex::from_str("840d9edffffffff").unwrap();
        let h3_boundary = cell.boundary();
        let boundary = Value::Array(
            h3_boundary
                .iter()
                .map(|coordinate| {
                    json!({
                        "latitude":coordinate.lat(),
                        "longitude":coordinate.lng()
                    })
                })
                .collect(),
        );
        let geometry = result(
            Operation::FromBoundary,
            json!({"boundary":boundary.clone()}),
            "geometry_out",
        );
        assert_eq!(geometry["type"], "MultiPolygon");
        assert_eq!(geometry["coordinates"].as_array().unwrap().len(), 2);
        assert!(to_geo(&geometry).unwrap().is_valid());

        let dissolved = result(
            Operation::FromPolygons,
            json!({"polygons":[{"exterior":boundary,"interiors":[]}]}),
            "geometry_out",
        );
        assert_eq!(dissolved["type"], "MultiPolygon");
        assert_eq!(dissolved["coordinates"].as_array().unwrap().len(), 2);
        assert!(to_geo(&dissolved).unwrap().is_valid());
    }

    #[test]
    fn geometry_predicates_respect_holes_boundaries_and_invalid_topology() {
        let polygon = json!({"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[1,3],[3,3],[3,1],[1,1]]]});
        assert_eq!(
            result(
                Operation::Contains,
                json!({"a":polygon,"b":point(2.0,2.0)}),
                "result"
            ),
            false
        );
        assert_eq!(
            result(
                Operation::Contains,
                json!({"a":polygon,"b":point(0.5,0.5)}),
                "result"
            ),
            true
        );
        assert_eq!(
            result(
                Operation::Contains,
                json!({"a":polygon,"b":point(0.0,2.0)}),
                "result"
            ),
            false
        );
        assert_eq!(
            result(
                Operation::Intersects,
                json!({"a":polygon,"b":point(0.0,2.0)}),
                "result"
            ),
            true
        );
        assert_eq!(
            result(
                Operation::Within,
                json!({"a":point(0.5,0.5),"b":polygon}),
                "result"
            ),
            true
        );
        let bowtie = json!({"type":"Polygon","coordinates":[[[0,0],[2,2],[0,2],[2,0],[0,0]]]});
        assert!(execute(Operation::Contains, &json!({"a":bowtie,"b":point(1.0,1.0)})).is_err());
        assert_eq!(
            result(
                Operation::ConvexHull,
                json!({"geometry":point(1.0,1.0)}),
                "geometry_out"
            ),
            point(1.0, 1.0)
        );
        let empty = json!({"type":"GeometryCollection","geometries":[]});
        assert!(execute(Operation::Centroid, &json!({"geometry":empty})).is_err());
    }

    #[test]
    fn intersection_preserves_lower_dimensional_results() {
        let polygon = square();
        let line = json!({"type":"LineString","coordinates":[[-1,0.5],[2,0.5]]});
        let intersection = result(
            Operation::Intersection,
            json!({"a":polygon,"b":line}),
            "geometry_out",
        );
        assert_eq!(intersection["type"], "LineString");
        assert_eq!(intersection["coordinates"], json!([[0.0, 0.5], [1.0, 0.5]]));

        let intersection_point = result(
            Operation::Intersection,
            json!({"a":square(),"b":point(0.25,0.25)}),
            "geometry_out",
        );
        assert_eq!(intersection_point, point(0.25, 0.25));
    }

    #[test]
    fn geometry_measurements_have_explicit_units_and_handle_antimeridian() {
        let input = json!({"a":point(0.0,0.0), "b":point(1.0,0.0)});
        assert_eq!(
            result(Operation::PlanarDistance, input.clone(), "distance"),
            1.0
        );
        let meters = result(Operation::GeodesicDistance, input, "distance")
            .as_f64()
            .unwrap();
        assert!((meters - 111_319.490_793).abs() < 0.01);
        let crossing = json!({"a":point(179.0,0.0), "b":point(-179.0,0.0)});
        let meters = result(Operation::GeodesicDistance, crossing, "distance")
            .as_f64()
            .unwrap();
        assert!((meters - 222_638.981_586).abs() < 0.01);
        let line = json!({"type":"LineString","coordinates":[[0,0],[1,0],[2,0]]});
        assert_eq!(
            result(Operation::PlanarLength, json!({"geometry":line}), "length"),
            2.0
        );
        let meters = result(
            Operation::GeodesicLength,
            json!({"geometry":line}),
            "length",
        )
        .as_f64()
        .unwrap();
        assert!((meters - 222_638.981_586).abs() < 0.01);
        assert_eq!(
            result(Operation::PlanarArea, json!({"geometry":square()}), "area"),
            1.0
        );
        assert_eq!(
            result(Operation::PlanarArea, json!({"geometry":line}), "area"),
            0.0
        );
        let area = result(
            Operation::GeodesicArea,
            json!({"geometry":square()}),
            "area",
        )
        .as_f64()
        .unwrap();
        assert!((area - 12_308_778_361.469).abs() < 1.0);
        assert!(
            execute(
                Operation::GeodesicDistance,
                &json!({"a":line,"b":point(0.0,0.0)})
            )
            .is_err()
        );
    }
}
