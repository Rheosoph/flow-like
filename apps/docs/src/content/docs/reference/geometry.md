---
title: Geometry
description: Store locations and shapes, validate their types, and query spatial data.
---

A **Geometry** variable holds a GeoJSON geometry object. Geometry pins and tokens
use orange (`#F97316`). Choose Geometry in the variable editor, optionally select
a subtype, and enter GeoJSON. The editor validates the value before saving it and
previews valid shapes on a map.

```json
{"type":"Point","coordinates":[13.405,52.52]}
```

Coordinates are **longitude, latitude**, in WGS 84 degrees. The example places a
point in Berlin. Reversing those numbers describes a different location.

## Values and subtypes

Supported subtypes are Point, LineString, Polygon, MultiPoint, MultiLineString,
MultiPolygon, and GeometryCollection. A concrete subtype can connect to an input
that accepts any Geometry. To connect an unrestricted Geometry to a Point input,
use **Validate Point**. Each subtype has a corresponding validating cast node.

FlowScript spells these types `geometry` and `geometry<Point>`. Arrays, sets, and
maps contain Geometry values independently of their subtype. A MultiPoint is one
Geometry value; an array of Points is a container of separate values. Set equality
compares values, not spatial equivalence.

The current profile accepts exactly two finite coordinates per position, with
longitude between -180 and 180 and latitude between -90 and 90. Polygon rings must
be closed and contain at least four positions. Import and editing normalize ring
winding. Spatial operation nodes also check topology before computing a result.

Null represents an unset value. Required inputs reject it. Empty multi-geometries
and GeometryCollections are supported; empty Point, LineString, Polygon, and empty
members inside multi-geometries are rejected. Values are limited to 1 MiB of JSON,
32 levels of nesting, and 100,000 positions. Operations that require pairwise
topology checks also apply a two-million-comparison work limit. Simplify dense
polygon rings or split large collections when an operation reaches that limit.

Native Geometry nodes run CPU work on a shared pool with at most eight workers,
bounded by the host's available CPU parallelism. Concurrent flows and nested polygon
overlays share this pool. Geometry computation does not create or enter a Tokio
runtime. Large independent scans and collection operations use parallel workers;
small loops remain sequential. Results retain input order and deterministic distance
tie handling, and parallel tasks share the operation's resource limits.

Cancellation releases the caller immediately when its execution future is dropped.
CPU work already running retains its worker admission slot until it finishes, so
cancelling repeated runs cannot bypass the concurrency bound. Computation errors and
unwinding panics return node errors. The Geometry worker layer uses sequential
computation on WebAssembly.

Feature and FeatureCollection wrappers are not Geometry values. Dedicated nodes
extract their geometries and properties or build the wrappers from Geometry values.
GeoJSON conversion retains valid foreign members and bounding boxes. WKT and WKB
preserve the shape but cannot carry those extra JSON members. WKT imports assert
WGS 84; WKB imports accept two-dimensional values and an optional EPSG:4326 SRID.
The Web Mercator conversion nodes use a Struct for EPSG:3857 coordinates because a
Geometry value always means WGS 84. Other projected coordinate systems and Z/M
dimensions remain unsupported.

## Operations and existing flows

The Geometry catalog provides native constructors for every supported subtype,
component access and decomposition, GeoJSON/WKT/WKB conversion, Feature adapters,
topological predicates, polygon Boolean operations, mixed-dimensional intersection,
validity diagnostics and limited repair. It also includes buffers, derived shapes,
line editing, simplification, affine transforms, triangulation, smoothing, encoded
polyline and geohash conversion, and H3 coverage adapters.

Planar operations use the longitude/latitude coordinate plane. Their distances and
lengths are degrees, and their areas are square degrees. Use **Split Line at
Antimeridian** before planar work on a crossing LineString. **Wrap Geometry
Longitude** accepts a GeoJSON-like Struct containing Points, MultiPoints, or nested
point-only collections and returns a Geometry. Connected shapes are rejected because
wrapping individual vertices would change their planar edges.

Use the explicitly named meter and square-meter operations for WGS 84 geodesic
measurements. Geodesic distance accepts Points. Geodesic length includes line
segments and polygon ring perimeters. Geodesic area accepts polygons smaller than
half the Earth and subtracts holes. Geodesic bearing, destination, interpolation,
densification, circles, and discrete LineString Fréchet distance are also available.

Reverse Geocode, Get Map Image, Lat/Lng to H3 Cell, and OSRM Nearest accept a Point
through their Geometry input. Route planning accepts start and end Points plus
optional waypoint Points. The OSRM table, trace matching, and trip nodes accept
ordered Point arrays. Search and reverse geocoding return Points; H3 boundary
nodes return polygons; routing nodes return route LineStrings and waypoint Points
alongside their existing result objects.

Geo nodes exchange spatial values through Geometry pins. Location measurements,
addresses, and route details remain available as result objects. There are no
duplicate coordinate Struct pins on these nodes.

When an older board is loaded, saved coordinate literals are converted to
Geometry. Connections to existing Struct data use conversion adapters so that
older data sources and consumers keep their original shape. Explicit adapters
also remain available for stored coordinate, boundary, polygon, route, and
location payloads.

Geometry-native H3 nodes also convert Polygon or MultiPolygon Geometry to bounded
cell coverage.
H3 coverage follows the profile's direct longitude edges. A region intended to cross
the antimeridian must already be represented as split polygon parts.
Route and location result objects retain their properties for downstream nodes.

## Tables and SQL

Create a scalar **geometry** column in Data Studio or declare `"type":"geometry"`
in a table schema. The column stores WKB with GeoArrow WGS 84 metadata. Ordinary
Binary columns remain byte arrays, and Struct columns remain objects. Geometry
previews depend on the declared Arrow metadata.

Insert or upsert validated GeoJSON into a declared geometry column. Direct Arrow
inserts require compatible GeoArrow metadata and validate the values. Native
GeoArrow query output can be inserted into a declared WKB column. SQL INSERT into
geometry tables, geometry UPDATE assignments, and geometry expression columns
are currently rejected. Use validated insert/upsert operations instead.

Application SQL sessions expose spatial functions, including `ST_Intersects`,
`ST_Contains`, and `ST_Centroid`. To produce a Geometry result from WKT, use the
explicit WGS 84 import helper:

```sql
SELECT ST_Centroid(
  flow_geomfromtext('LINESTRING(10 20,20 30)')
) AS center;
```

`flow_geomfromtext` validates longitude/latitude coordinates and attaches WGS 84
metadata. It does not transform coordinates. `ST_GeomFromText` carries unknown CRS,
so its results require an explicit WGS 84 assertion before conversion to a flow
Geometry. Bind WKT parameters as strings, for example
`ST_Intersects(geom, flow_geomfromtext($1))`.

SQL spatial measurements are planar. Application SQL evaluates spatial predicates
above the Lance scan and applies limits after filtering. Direct Lance spatial
filters can use an RTree index. Do not infer index use from the presence of an
index alone; inspect the query plan for the query path being used.

## Client compatibility

Remote Geometry creation and execution are enabled by default. Geometry requires
board format version 2. Boards without a stored `format_version` use version 1;
loaders also infer version 2 for existing Geometry boards. Board format versions
are independent of app release numbers and compiled artifact versions.

Clients advertise their highest supported version with
`x-flow-like-board-format: 2`. A missing header means version 1. The endpoint
`/apps/{app}/board/capabilities` returns `{ "board_format_version": 2 }`, which
lets clients enable features supported by the backend.

Clients that cannot support a board receive `BOARD_FORMAT_UPGRADE_REQUIRED`
(HTTP 426) before receiving or changing it. Realtime sessions use rooms separated
by the negotiated board format. API, executor, and signaling deployments must
support version 2 before running Geometry flows. Existing Geometry boards retain
that requirement if a deployment is downgraded.
