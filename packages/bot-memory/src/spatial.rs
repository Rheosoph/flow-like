//! GeoJSON geometry validation and location queries. Coordinates are WGS84 longitude, latitude.
//! Area intersection includes boundaries and excludes holes. Distances use a spherical Earth.

use flow_like_types::{Result, anyhow, bail};
use geo::{
    Closest, Coord, Distance, Geometry, GeometryCollection, Haversine, HaversineClosestPoint,
    Intersects, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon, Validation,
};
use serde_json::Value;

const MAX_COORDINATES: usize = 10_000;
const MAX_DEPTH: usize = 16;

fn array(value: &Value) -> Result<&[Value]> {
    value
        .as_array()
        .filter(|values| !values.is_empty())
        .map(Vec::as_slice)
        .ok_or_else(|| anyhow!("GeoJSON coordinates and geometry lists must be nonempty arrays"))
}

fn coordinate(value: &Value, remaining: &mut usize) -> Result<Coord<f64>> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or_else(|| anyhow!("GeoJSON must contain at most {MAX_COORDINATES} coordinates"))?;
    let values = array(value)?;
    if !(2..=3).contains(&values.len()) {
        bail!("A GeoJSON position must be [longitude, latitude] with optional altitude");
    }
    let numbers = values
        .iter()
        .map(|value| {
            value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| anyhow!("GeoJSON coordinates must be finite numbers"))
        })
        .collect::<Result<Vec<_>>>()?;
    if !(-180.0..=180.0).contains(&numbers[0]) || !(-90.0..=90.0).contains(&numbers[1]) {
        bail!("GeoJSON longitude must be between -180 and 180 and latitude between -90 and 90");
    }
    Ok(Coord {
        x: numbers[0],
        y: numbers[1],
    })
}

fn line(value: &Value, ring: bool, remaining: &mut usize) -> Result<LineString<f64>> {
    let positions = array(value)?
        .iter()
        .map(|value| coordinate(value, remaining))
        .collect::<Result<Vec<_>>>()?;
    if positions.len() < if ring { 4 } else { 2 } {
        bail!("GeoJSON lines need at least two positions and polygon rings at least four");
    }
    if ring && positions.first() != positions.last() {
        bail!("GeoJSON polygon rings must be closed by repeating their first position");
    }
    if positions
        .windows(2)
        .any(|pair| (pair[0].x - pair[1].x).abs() >= 180.0)
    {
        bail!(
            "Split lines and polygons crossing the antimeridian into Multi geometries at longitude -180/180"
        );
    }
    Ok(LineString::new(positions))
}

fn polygon(value: &Value, remaining: &mut usize) -> Result<Polygon<f64>> {
    let mut rings = array(value)?
        .iter()
        .map(|value| line(value, true, remaining))
        .collect::<Result<Vec<_>>>()?
        .into_iter();
    Ok(Polygon::new(rings.next().unwrap(), rings.collect()))
}

fn geometry(value: &Value, depth: usize, remaining: &mut usize) -> Result<Geometry<f64>> {
    if depth > MAX_DEPTH {
        bail!("GeoJSON GeometryCollection nesting must not exceed {MAX_DEPTH}");
    }
    if value.get("crs").is_some() {
        bail!("GeoJSON must use WGS84 longitude/latitude without a custom crs");
    }
    let coordinates = &value["coordinates"];
    Ok(match value["type"].as_str() {
        Some("Point") => Point::from(coordinate(coordinates, remaining)?).into(),
        Some("MultiPoint") => MultiPoint::new(
            array(coordinates)?
                .iter()
                .map(|value| coordinate(value, remaining).map(Point::from))
                .collect::<Result<Vec<_>>>()?,
        )
        .into(),
        Some("LineString") => line(coordinates, false, remaining)?.into(),
        Some("MultiLineString") => MultiLineString::new(
            array(coordinates)?
                .iter()
                .map(|value| line(value, false, remaining))
                .collect::<Result<Vec<_>>>()?,
        )
        .into(),
        Some("Polygon") => polygon(coordinates, remaining)?.into(),
        Some("MultiPolygon") => MultiPolygon::new(
            array(coordinates)?
                .iter()
                .map(|value| polygon(value, remaining))
                .collect::<Result<Vec<_>>>()?,
        )
        .into(),
        Some("GeometryCollection") => Geometry::GeometryCollection(GeometryCollection::new_from(
            array(&value["geometries"])?
                .iter()
                .map(|value| geometry(value, depth + 1, remaining))
                .collect::<Result<Vec<_>>>()?,
        )),
        _ => bail!(
            "Use a GeoJSON geometry with type and coordinates (or GeometryCollection with geometries), not a Feature or FeatureCollection"
        ),
    })
}

fn parse(value: &Value) -> Result<Geometry<f64>> {
    let mut remaining = MAX_COORDINATES;
    let geometry = geometry(value, 0, &mut remaining)?;
    geometry
        .check_validation()
        .map_err(|error| anyhow!("Invalid GeoJSON geometry: {error}"))?;
    Ok(geometry)
}

/// Checks a nonempty GeoJSON geometry, including polygon topology and coordinate bounds.
pub fn validate_geojson(value: &Value) -> Result<()> {
    parse(value).map(|_| ())
}

fn point(value: &Value) -> Result<Point<f64>> {
    match parse(value)? {
        Geometry::Point(point) => Ok(point),
        _ => bail!("The distance reference geojson must be a Point"),
    }
}

pub fn validate_point(value: &Value) -> Result<()> {
    point(value).map(|_| ())
}

fn area(value: &Value) -> Result<Geometry<f64>> {
    let geometry = parse(value)?;
    if !matches!(geometry, Geometry::Polygon(_) | Geometry::MultiPolygon(_)) {
        bail!("The area must be a GeoJSON Polygon or MultiPolygon");
    }
    Ok(geometry)
}

pub fn validate_area(value: &Value) -> Result<()> {
    area(value).map(|_| ())
}

/// Shortest spherical distance in meters from a geometry to a reference Point. Points inside
/// polygons have distance zero; points inside holes measure to the hole boundary.
pub fn distance_m(value: &Value, reference: &Value) -> Result<f64> {
    let geometry = parse(value)?;
    let reference = point(reference)?;
    let distance = match geometry.haversine_closest_point(&reference) {
        Closest::Intersection(_) => 0.0,
        Closest::SinglePoint(closest) => Haversine.distance(reference, closest),
        Closest::Indeterminate => bail!("Cannot determine a distance to this geometry"),
    };
    if !distance.is_finite() {
        bail!("Cannot determine a finite distance to this geometry");
    }
    Ok(distance)
}

/// Whether the geometry intersects a Polygon or MultiPolygon, including their boundaries.
pub fn intersects_area(value: &Value, boundary: &Value) -> Result<bool> {
    Ok(parse(value)?.intersects(&area(boundary)?))
}

/// The derived two-dimensional geometry stored in Lance's GeoArrow column. The original
/// GeoJSON stays on the fact, including optional altitude and foreign members.
pub(crate) fn index_geometry(value: &Value) -> Result<Value> {
    validate_geojson(value)?;
    fn positions(value: &Value) -> Value {
        let values = value.as_array().expect("validated coordinates");
        if values.first().is_some_and(Value::is_number) {
            Value::Array(values.iter().take(2).cloned().collect())
        } else {
            Value::Array(values.iter().map(positions).collect())
        }
    }
    fn geometry(value: &Value) -> Value {
        if value["type"] == "GeometryCollection" {
            serde_json::json!({
                "type": "GeometryCollection",
                "geometries": value["geometries"].as_array().expect("validated geometries")
                    .iter().map(geometry).collect::<Vec<_>>()
            })
        } else {
            serde_json::json!({"type": value["type"], "coordinates": positions(&value["coordinates"])})
        }
    }
    Ok(geometry(value))
}

/// A validated area as WKT for the database's spatial predicate. Only parsed numbers enter
/// the generated expression; caller-supplied text never becomes SQL syntax.
pub(crate) fn area_wkt(value: &Value) -> Result<String> {
    fn ring(ring: &LineString<f64>) -> String {
        format!(
            "({})",
            ring.0
                .iter()
                .map(|position| format!("{} {}", position.x, position.y))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
    fn polygon(polygon: &Polygon<f64>) -> String {
        format!(
            "({})",
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .map(ring)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
    match area(value)? {
        Geometry::Polygon(shape) => Ok(format!("POLYGON {}", polygon(&shape))),
        Geometry::MultiPolygon(shapes) => Ok(format!(
            "MULTIPOLYGON ({})",
            shapes.0.iter().map(polygon).collect::<Vec<_>>().join(",")
        )),
        _ => unreachable!("validated area"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn point(lon: f64, lat: f64) -> Value {
        json!({"type": "Point", "coordinates": [lon, lat]})
    }

    fn holed_area() -> Value {
        json!({"type":"Polygon","coordinates":[
            [[0,0],[4,0],[4,4],[0,4],[0,0]],
            [[1,1],[1,3],[3,3],[3,1],[1,1]]
        ]})
    }

    #[test]
    fn distances_use_meters_and_wrap_point_longitudes() {
        let equator_degree = distance_m(&point(1.0, 0.0), &point(0.0, 0.0)).unwrap();
        assert!((equator_degree - 111_195.08).abs() < 1.0);
        let across_dateline = distance_m(&point(-179.9, 0.0), &point(179.9, 0.0)).unwrap();
        assert!((across_dateline - 22_239.02).abs() < 1.0);
    }

    #[test]
    fn area_queries_respect_holes_boundaries_and_crossing_lines() {
        let area = holed_area();
        assert!(intersects_area(&point(0.5, 0.5), &area).unwrap());
        assert!(!intersects_area(&point(2.0, 2.0), &area).unwrap());
        assert!(intersects_area(&point(1.0, 2.0), &area).unwrap());
        assert_eq!(distance_m(&area, &point(0.5, 0.5)).unwrap(), 0.0);
        assert!(distance_m(&area, &point(2.0, 2.0)).unwrap() > 100_000.0);
        let crossing = json!({"type":"LineString","coordinates":[[-1,0.5],[5,0.5]]});
        assert!(intersects_area(&crossing, &area).unwrap());
    }

    #[test]
    fn malformed_empty_and_ambiguous_geometries_are_rejected() {
        for invalid in [
            point(181.0, 0.0),
            point(0.0, 91.0),
            json!({"type":"Point","coordinates": [1]}),
            json!({"type":"MultiPoint","coordinates": []}),
            json!({"type":"GeometryCollection","geometries": []}),
            json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}),
            json!({"type":"Polygon","coordinates":[[[0,0],[2,2],[0,2],[2,0],[0,0]]]}),
            json!({"type":"LineString","coordinates":[[179,0],[-179,0]]}),
            json!({"type":"Feature","geometry": point(0.0,0.0)}),
        ] {
            assert!(validate_geojson(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn collections_and_split_antimeridian_areas_work() {
        let split = json!({"type":"MultiPolygon","coordinates":[
            [[[179,-1],[180,-1],[180,1],[179,1],[179,-1]]],
            [[[-180,-1],[-179,-1],[-179,1],[-180,1],[-180,-1]]]
        ]});
        assert!(intersects_area(&point(-179.5, 0.0), &split).unwrap());
        assert!(!intersects_area(&point(0.0, 0.0), &split).unwrap());
        let collection =
            json!({"type":"GeometryCollection","geometries":[point(10.0,0.0),point(0.0,0.0)]});
        assert_eq!(distance_m(&collection, &point(0.0, 0.0)).unwrap(), 0.0);
    }

    #[test]
    fn derived_index_geometries_drop_altitude_and_keep_holes() {
        let original = json!({"type":"GeometryCollection", "extra":"kept only on the fact", "geometries":[
            {"type":"Point","coordinates":[13.4,52.5,120]}, holed_area()
        ]});
        let indexed = index_geometry(&original).unwrap();
        assert_eq!(indexed["geometries"][0]["coordinates"], json!([13.4, 52.5]));
        assert!(indexed.get("extra").is_none());
        assert_eq!(indexed["geometries"][1], holed_area());
        assert_eq!(
            area_wkt(&holed_area()).unwrap(),
            "POLYGON ((0 0,4 0,4 4,0 4,0 0),(1 1,1 3,3 3,3 1,1 1))"
        );
    }
}
