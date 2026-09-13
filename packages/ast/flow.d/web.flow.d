// Web — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace auth {
    // === Web/Auth ===

    /**
     * Creates REST auth that requires a configured API key header.
     * @node api_key_auth @alias apiKeyAuth
     * @param header (optional) — Header that carries the API key
     * @param key — Expected API key
     * @returns auth — API key auth config
     */
    function apiKey({ header?: string, key: string }): Struct;

    /**
     * Creates REST auth that requires HTTP Basic credentials.
     * @node basic_auth @alias basicAuth
     * @param username — Expected username
     * @param password — Expected password
     * @returns auth — Basic auth config
     */
    function basic({ username: string, password: string }): Struct;

    /**
     * Creates REST auth that requires a static Authorization bearer token.
     * @node bearer_token_auth @alias bearerTokenAuth
     * @param token — Expected bearer token
     * @returns auth — Bearer token auth config
     */
    function bearer({ token: string }): Struct;

    /**
     * Creates REST auth that verifies an HMAC-SHA256 request signature.
     * @node hmac_sha256_auth @alias hmacSha256Auth
     * @param secret — Shared HMAC secret
     * @param signatureHeader (optional) — Header that carries the lowercase hex HMAC signature
     * @param timestampHeader (optional) — Header that carries the Unix timestamp in seconds
     * @param maxSkewSeconds (optional) — Allowed timestamp skew in seconds; zero disables timestamp freshness checks
     * @returns auth — HMAC auth config
     */
    function hmacSha256({ secret: string, signatureHeader?: string, timestampHeader?: string, maxSkewSeconds?: int }): Struct;

    /**
     * Creates OAuth bearer auth from a JWKS JSON FlowPath loaded when the server starts.
     * @node oauth_jwks_file_auth @alias oauthJwksFileAuth
     * @param jwksFlowPath — JWKS JSON file FlowPath
     * @param issuer — Required token issuer. Empty disables issuer validation.
     * @param audience — Required token audience. Empty disables audience validation.
     * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
     * @returns auth — OAuth auth config
     */
    function oauthJwksFile({ jwksFlowPath: Struct, issuer: string, audience: string, requiredScopes: string[] }): Struct;

    /**
     * Creates OAuth bearer auth that fetches a JWKS endpoint once when the server starts.
     * @node oauth_jwks_url_auth @alias oauthJwksUrlAuth
     * @param jwksUrl — JWKS endpoint URL
     * @param issuer — Required token issuer. Empty disables issuer validation.
     * @param audience — Required token audience. Empty disables audience validation.
     * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
     * @returns auth — OAuth auth config
     */
    function oauthJwksUrl({ jwksUrl: string, issuer: string, audience: string, requiredScopes: string[] }): Struct;

    /**
     * Creates OAuth bearer auth by discovering the JWKS URI from an OpenID Connect issuer.
     * @node oidc_discovery_auth @alias oidcDiscoveryAuth
     * @param issuer — OIDC issuer URL. The server fetches /.well-known/openid-configuration.
     * @param audience — Required token audience. Empty disables audience validation.
     * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
     * @returns auth — OIDC auth config
     */
    function oidcDiscovery({ issuer: string, audience: string, requiredScopes: string[] }): Struct;
}

declare namespace camera {
    // === Web/Camera ===

    /**
     * Captures a frame from an IP camera
     * @node web_camera_grab_frame @alias webCameraGrabFrame
     * @param request — The HTTP request to perform
     * @returns image — The captured image frame
     * @impure has side effects / drives control flow
     */
    function grabFrame({ request: Struct }): Struct;

    /**
     * Captures one frame from an RTSP camera stream
     * @node web_camera_grab_rtsp_frame @alias webCameraGrabRtspFrame
     * @param rtspUrl — RTSP or RTSPS stream URL
     * @param transport (optional) — RTSP RTP transport protocol
     * @param timeoutMs (optional) — Maximum time in milliseconds to connect and decode a frame
     * @param maxFrames (optional) — Maximum video frames to inspect before failing
     * @returns image — The captured RTSP frame
     * @returns errorMessage — Readable capture error
     * @impure has side effects / drives control flow
     */
    function grabRtspFrame({ rtspUrl: string, transport?: string, timeoutMs?: int, maxFrames?: int }): { image: Struct, errorMessage: string };
}

declare namespace geo {
    // === Web/Geo/Location ===

    /**
     * Gets a location measurement from the local device, or from the invoking frontend for a remote Event. Requires location permission and an active app.
     * @node geo_get_current_location @alias geoGetCurrentLocation
     * @param highAccuracy (optional) — Request higher accuracy, which can use more power and take longer
     * @param maximumAgeSeconds (optional) — Maximum age of a cached measurement in seconds, from 0 to 300. Zero requests a fresh measurement
     * @param timeoutSeconds (optional) — Seconds allowed for permission and location acquisition, from 1 to 120
     * @returns geometry — WGS 84 Point in longitude, latitude order
     * @returns location — Measurement with accuracy in meters, timestamp in Unix milliseconds, optional altitude in meters, speed in meters per second, and heading in degrees
     * @returns error — Structured error code and message
     * @impure has side effects / drives control flow
     */
    function getCurrentLocation({ highAccuracy?: bool, maximumAgeSeconds?: int, timeoutSeconds?: int }): { geometry: geometry<Point>, location: Struct, error: Struct };

    // === Web/Geo/Map ===

    /**
     * Fetches a static map image for the given coordinates using OpenStreetMap tiles. Returns a satellite/standard map image centered on the location.
     * @node geo_get_map_image @alias geoGetMapImage
     * @param geometry — Point at the map center
     * @param zoom (optional) — Map zoom level (1-19). Higher values show more detail. Default: 15
     * @param width (optional) — Image width in pixels. Default: 512
     * @param height (optional) — Image height in pixels. Default: 512
     * @param style (optional) — Map style to use
     * @returns image — The fetched map image
     * @impure has side effects / drives control flow
     */
    function getMapImage({ geometry: geometry<Point>, zoom?: int, width?: int, height?: int, style?: string }): Struct;

    // === Web/Geo/Routing ===

    /**
     * Snaps noisy GPS traces to the road network using OSRM map matching.
     * @node geo_osrm_match_trace @alias geoOsrmMatchTrace
     * @param geometries — Ordered Point geometries
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param timestamps (optional) — Optional UNIX timestamps for each coordinate (seconds)
     * @param radiuses (optional) — Optional search radiuses in meters for each coordinate
     * @param gaps (optional) — How to handle gaps: split or ignore
     * @param tidy (optional) — Simplify the matched geometry
     * @param baseUrl (optional) — OSRM server base URL
     * @returns matchings — Matched routes for the trace
     * @returns primaryMatching — Primary matched route
     * @returns tracepoints — Tracepoints mapped to the road network
     * @returns geometryOut — Primary route as a LineString geometry. Unset when no route is found.
     * @returns routeGeometries — LineString geometries for all returned routes, with the primary route first.
     * @impure has side effects / drives control flow
     */
    function osrmMatchTrace({ geometries: geometry<Point>[], profile?: Struct, timestamps?: int[], radiuses?: float[], gaps?: string, tidy?: bool, baseUrl?: string }): { matchings: Struct[], primaryMatching: Struct, tracepoints: Struct[], geometryOut: geometry<LineString>, routeGeometries: geometry<LineString>[] };

    /**
     * Finds the nearest routable point(s) to a coordinate using OSRM.
     * @node geo_osrm_nearest @alias geoOsrmNearest
     * @param geometry — Point geometry to snap to the road network
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param number (optional) — Maximum number of nearest points to return (1-50)
     * @param baseUrl (optional) — OSRM server base URL
     * @returns nearest — The closest routable point
     * @returns waypoints — List of nearest routable points
     * @returns geometryOut — Closest routable Point geometry. Unset when no point is found.
     * @returns waypointGeometries — Nearest routable Point geometries in the same order as Waypoints.
     * @impure has side effects / drives control flow
     */
    function osrmNearest({ geometry: geometry<Point>, profile?: Struct, number?: int, baseUrl?: string }): { nearest: Struct, waypoints: Struct[], geometryOut: geometry<Point>, waypointGeometries: geometry<Point>[] };

    /**
     * Computes travel time and distance matrices between coordinates using OSRM.
     * @node geo_osrm_table @alias geoOsrmTable
     * @param geometries — Ordered Point geometries
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param sources (optional) — Optional indices of source coordinates
     * @param destinations (optional) — Optional indices of destination coordinates
     * @param includeDurations (optional) — Return travel time matrix
     * @param includeDistances (optional) — Return travel distance matrix
     * @param baseUrl (optional) — OSRM server base URL
     * @returns durations — Matrix of travel times in seconds
     * @returns distances — Matrix of travel distances in meters
     * @returns result — Matrix result containing durations and distances
     * @impure has side effects / drives control flow
     */
    function osrmTable({ geometries: geometry<Point>[], profile?: string, sources?: int[], destinations?: int[], includeDurations?: bool, includeDistances?: bool, baseUrl?: string }): { durations: Struct[], distances: Struct[], result: Struct };

    /**
     * Fetches vector map tiles (MVT) from an OSRM server.
     * @node geo_osrm_tile @alias geoOsrmTile
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param z (optional) — Tile zoom level
     * @param x (optional) — Tile X coordinate
     * @param y (optional) — Tile Y coordinate
     * @param path — Destination path for the MVT tile
     * @param baseUrl (optional) — OSRM server base URL
     * @returns tilePath — Stored tile path
     * @returns contentType — Content type returned by the server
     * @impure has side effects / drives control flow
     */
    function osrmTile({ profile?: Struct, z?: int, x?: int, y?: int, path: Struct, baseUrl?: string }): { tilePath: Struct, contentType: string };

    /**
     * Plans the shortest round trip through multiple coordinates using OSRM.
     * @node geo_osrm_trip @alias geoOsrmTrip
     * @param geometries — Ordered Point geometries
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param roundtrip (optional) — Return to the starting point
     * @param source (optional) — Source location: any, first, or last
     * @param destination (optional) — Destination location: any, first, or last
     * @param baseUrl (optional) — OSRM server base URL
     * @returns trip — Primary trip result
     * @returns trips — All trip results returned by OSRM
     * @returns waypoints — Optimized trip waypoints
     * @returns distance — Total trip distance in meters
     * @returns duration — Total trip duration in seconds
     * @returns geometryOut — Primary route as a LineString geometry. Unset when no route is found.
     * @returns routeGeometries — LineString geometries for all returned routes, with the primary route first.
     * @returns waypointGeometries — Snapped Point geometries in the same order as Waypoints. Use waypoint_index in Waypoints for the optimized visit order.
     * @impure has side effects / drives control flow
     */
    function osrmTrip({ geometries: geometry<Point>[], profile?: Struct, roundtrip?: bool, source?: string, destination?: string, baseUrl?: string }): { trip: Struct, trips: Struct[], waypoints: Struct[], distance: float, duration: float, geometryOut: geometry<LineString>, routeGeometries: geometry<LineString>[], waypointGeometries: geometry<Point>[] };

    /**
     * Plans a route between two points using the OSRM routing service. Returns turn-by-turn directions, distance, and duration.
     * @node geo_plan_route @alias geoPlanRoute
     * @param startGeometry — Starting Point geometry for the route
     * @param endGeometry — Ending Point geometry for the route
     * @param waypointGeometries (optional) — Optional intermediate Point geometries in visit order
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param alternatives (optional) — Request alternative routes
     * @returns route — The primary calculated route
     * @returns alternativesOut — Alternative routes if requested
     * @returns distance — Total route distance in meters
     * @returns duration — Estimated travel time in seconds
     * @returns geometryOut — Primary route as a LineString geometry. Unset when no route is found.
     * @returns routeGeometries — LineString geometries for all returned routes, with the primary route first.
     * @impure has side effects / drives control flow
     */
    function planRoute({ startGeometry: geometry<Point>, endGeometry: geometry<Point>, waypointGeometries?: geometry<Point>[], profile?: string, alternatives?: bool }): { route: Struct, alternativesOut: Struct, distance: float, duration: float, geometryOut: geometry<LineString>, routeGeometries: geometry<LineString>[] };

    // === Web/Geo/Search ===

    /**
     * Converts geographic coordinates to a human-readable address using the Nominatim service (OpenStreetMap).
     * @node geo_reverse_geocode @alias geoReverseGeocode
     * @param geometry — Point to look up
     * @param zoom (optional) — Level of detail for the address (0-18). Higher = more specific. Default: 18
     * @returns result — The reverse geocoding result with address details
     * @returns displayName — The full formatted address string
     * @returns geometryOut — Point returned by the geocoding service
     * @impure has side effects / drives control flow
     */
    function reverseGeocode({ geometry: geometry<Point>, zoom?: int }): { result: Struct, displayName: string, geometryOut: geometry<Point> };

    /**
     * Searches for a location by name or address using the Nominatim geocoding service (OpenStreetMap). Returns matching locations with coordinates.
     * @node geo_search_location @alias geoSearchLocation
     * @param query (optional) — The search query (address, place name, etc.)
     * @param limit (optional) — Maximum number of results to return. Default: 5
     * @param countryCodes (optional) — Optional comma-separated list of country codes to limit search (e.g., 'de,at,ch')
     * @returns results — Array of search results with coordinates
     * @returns firstResult — The first/best matching result (if any)
     * @returns geometryOut — Point for the first match. Unset when no location matches.
     * @returns geometries — Points for all matches, in the same order as Results
     * @impure has side effects / drives control flow
     */
    function searchLocation({ query?: string, limit?: int, countryCodes?: string }): { results: Struct[], firstResult: Struct, geometryOut: geometry<Point>, geometries: geometry<Point>[] };
}

declare namespace geometry {
    // === Web/Geo/Geometry ===

    /**
     * Applies x' = a*x + b*y + x_offset and y' = d*x + e*y + y_offset. The matrix must be invertible.
     * @node geometry_affine_transform @receiver geometry @alias geometryAffineTransform
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.affineTransform(...)`)
     * @param a — a
     * @param b — b
     * @param xOffset — x_offset
     * @param d — d
     * @param e — e
     * @param yOffset — y_offset
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function affineTransform(this: geometry, { geometry: geometry, a: float, b: float, xOffset: float, d: float, e: float, yOffset: float }): geometry;

    /**
     * Subtracts Polygon or MultiPolygon B from A in the longitude/latitude coordinate plane. Returns a MultiPolygon.
     * @node geometry_difference @receiver a @alias geometryDifference
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.booleanDifference(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function booleanDifference(this: geometry, { a: geometry, b: geometry }): geometry<MultiPolygon>;

    /**
     * Combines Polygon or MultiPolygon regions in the longitude/latitude coordinate plane. Returns a MultiPolygon.
     * @node geometry_union @receiver a @alias geometryUnion
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.booleanUnion(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function booleanUnion(this: geometry, { a: geometry, b: geometry }): geometry<MultiPolygon>;

    /**
     * Returns the planar topological boundary. Point boundaries are empty, line boundaries are endpoint MultiPoints and polygon boundaries are MultiLineStrings. GeometryCollections containing both lineal and polygonal members are unsupported.
     * @node geometry_boundary @receiver geometry @alias geometryBoundary
     * @param geometry — Input geometry (receiver: `this` in `x.boundary(...)`)
     * @returns geometryOut — Result geometry
     */
    function boundary(this: geometry, { geometry: geometry }): geometry;

    /**
     * Returns minimum and maximum longitude and latitude using a planar coordinate envelope. Empty geometries have no bounds.
     * @node geometry_bounds @receiver geometry @alias geometryBounds
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.bounds(...)`)
     * @returns minLongitude — Coordinate in degrees
     * @returns minLatitude — Coordinate in degrees
     * @returns maxLongitude — Coordinate in degrees
     * @returns maxLatitude — Coordinate in degrees
     */
    function bounds(this: geometry, { geometry: geometry }): { minLongitude: float, minLatitude: float, maxLongitude: float, maxLatitude: float };

    /**
     * Validates a geometry as GeometryCollection and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_geometry_collection @receiver geometry @alias geometryCastGeometryCollection
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castGeometryCollection(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castGeometryCollection(this: geometry, { geometry: geometry }): geometry<GeometryCollection>;

    /**
     * Validates a geometry as LineString and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_line_string @receiver geometry @alias geometryCastLineString
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castLineString(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castLineString(this: geometry, { geometry: geometry }): geometry<LineString>;

    /**
     * Validates a geometry as MultiLineString and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_multi_line_string @receiver geometry @alias geometryCastMultiLineString
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castMultiLineString(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castMultiLineString(this: geometry, { geometry: geometry }): geometry<MultiLineString>;

    /**
     * Validates a geometry as MultiPoint and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_multi_point @receiver geometry @alias geometryCastMultiPoint
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castMultiPoint(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castMultiPoint(this: geometry, { geometry: geometry }): geometry<MultiPoint>;

    /**
     * Validates a geometry as MultiPolygon and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_multi_polygon @receiver geometry @alias geometryCastMultiPolygon
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castMultiPolygon(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castMultiPolygon(this: geometry, { geometry: geometry }): geometry<MultiPolygon>;

    /**
     * Validates a geometry as Point and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_point @receiver geometry @alias geometryCastPoint
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castPoint(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castPoint(this: geometry, { geometry: geometry }): geometry<Point>;

    /**
     * Validates a geometry as Polygon and returns it with an explicit subtype. Incompatible values fail the node.
     * @node geometry_cast_polygon @receiver geometry @alias geometryCastPolygon
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.castPolygon(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function castPolygon(this: geometry, { geometry: geometry }): geometry<Polygon>;

    /**
     * Computes the centroid in the longitude/latitude coordinate plane. Empty geometries have no centroid.
     * @node geometry_centroid @receiver geometry @alias geometryCentroid
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.centroid(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function centroid(this: geometry, { geometry: geometry }): geometry<Point>;

    /**
     * Returns the portions of a LineString or MultiLineString inside a Polygon or MultiPolygon mask, including its boundary.
     * @node geometry_clip_line @receiver line @alias geometryClipLine
     * @param line — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.clipLine(...)`)
     * @param mask — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function clipLine(this: geometry, { line: geometry, mask: geometry }): geometry<MultiLineString>;

    /**
     * Returns the point on a geometry nearest to an input Point in the longitude/latitude coordinate plane.
     * @node geometry_closest_point @receiver geometry @alias geometryClosestPoint
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.closestPoint(...)`)
     * @param point — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function closestPoint(this: geometry, { geometry: geometry, point: geometry<Point> }): geometry<Point>;

    /**
     * Builds a planar concave hull around all input positions. Larger positive concavity values produce less detailed hulls. Degenerate inputs return their natural Point or LineString hull.
     * @node geometry_concave_hull @receiver geometry @alias geometryConcaveHull
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.concaveHull(...)`)
     * @param concavity — concavity
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function concaveHull(this: geometry, { geometry: geometry, concavity: float }): geometry;

    /**
     * Tests whether geometry A contains B in the longitude/latitude coordinate plane. A point on a polygon boundary is not contained.
     * @node geometry_contains @receiver a @alias geometryContains
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.contains(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function contains(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Computes the smallest convex geometry containing the input in the longitude/latitude coordinate plane. A lower-dimensional input produces a Point or LineString.
     * @node geometry_convex_hull @receiver geometry @alias geometryConvexHull
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.convexHull(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function convexHull(this: geometry, { geometry: geometry }): geometry;

    /**
     * Tests whether every point of A lies in the interior or boundary of B using DE-9IM semantics.
     * @node geometry_covered_by @receiver a @alias geometryCoveredBy
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.coveredBy(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function coveredBy(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Tests whether every point of B lies in the interior or boundary of A using DE-9IM semantics.
     * @node geometry_covers @receiver a @alias geometryCovers
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.covers(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function covers(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Tests whether the inputs cross using DE-9IM semantics.
     * @node geometry_crosses @receiver a @alias geometryCrosses
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.crosses(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function crosses(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Tests whether the shortest planar distance between two non-empty geometries is at most the supplied coordinate-degree distance.
     * @node geometry_d_within @receiver a @alias geometryDWithin
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.dWithin(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @param distance (optional) — Maximum planar distance in coordinate degrees
     * @returns result — Whether the geometries are within the requested distance
     */
    function dWithin(this: geometry, { a: geometry, b: geometry, distance?: float }): bool;

    /**
     * Decodes a geohash into its center Point and rectangular bounding Polygon.
     * @node geometry_decode_geohash @alias geometryDecodeGeohash
     * @param geohash — Geohash text
     * @returns center — Geohash cell center
     * @returns bounds — Geohash cell bounds
     */
    function decodeGeohash({ geohash: string }): { center: geometry<Point>, bounds: geometry<Polygon> };

    /**
     * Decodes a Google encoded polyline into a validated WGS 84 LineString.
     * @node geometry_decode_polyline @alias geometryDecodePolyline
     * @param polyline — Encoded polyline text
     * @param precision (optional) — Decimal coordinate digits, from 0 through 10
     * @returns geometryOut — Decoded LineString
     */
    function decodePolyline({ polyline: string, precision?: int }): geometry<LineString>;

    /**
     * Adds points along WGS 84 geodesics so each segment is no longer than the positive maximum length in meters.
     * @node geometry_densify_geodesic @receiver geometry @alias geometryDensifyGeodesic
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.densifyGeodesic(...)`)
     * @param maxLength — max_length
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function densifyGeodesic(this: geometry<LineString>, { geometry: geometry<LineString>, maxLength: float }): geometry<LineString>;

    /**
     * Adds positions so each planar LineString segment is no longer than the positive maximum length in coordinate degrees.
     * @node geometry_densify_planar @receiver geometry @alias geometryDensifyPlanar
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.densifyPlanar(...)`)
     * @param maxLength — max_length
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function densifyPlanar(this: geometry<LineString>, { geometry: geometry<LineString>, maxLength: float }): geometry<LineString>;

    /**
     * Tests whether the inputs share no point using DE-9IM semantics.
     * @node geometry_disjoint @receiver a @alias geometryDisjoint
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.disjoint(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function disjoint(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Encodes a Geometry Point as a geohash with 1 through 12 characters.
     * @node geometry_encode_geohash @receiver geometry @alias geometryEncodeGeohash
     * @param geometry — Point to encode (receiver: `this` in `x.encodeGeohash(...)`)
     * @param precision (optional) — Geohash length from 1 through 12
     * @returns geohash — Encoded geohash
     */
    function encodeGeohash(this: geometry<Point>, { geometry: geometry<Point>, precision?: int }): string;

    /**
     * Encodes a LineString with the Google encoded polyline algorithm. Precision is the number of decimal coordinate digits.
     * @node geometry_encode_polyline @receiver geometry @alias geometryEncodePolyline
     * @param geometry — LineString to encode (receiver: `this` in `x.encodePolyline(...)`)
     * @param precision (optional) — Decimal coordinate digits, from 0 through 10
     * @returns polyline — Encoded polyline text
     */
    function encodePolyline(this: geometry<LineString>, { geometry: geometry<LineString>, precision?: int }): string;

    /**
     * Returns the last Point of a LineString.
     * @node geometry_end_point @receiver geometry @alias geometryEndPoint
     * @param geometry — LineString to inspect (receiver: `this` in `x.endPoint(...)`)
     * @returns geometryOut — Last Point
     */
    function endPoint(this: geometry<LineString>, { geometry: geometry<LineString> }): geometry<Point>;

    /**
     * Returns the axis-aligned planar bounds as a Polygon, LineString, or Point according to the input dimension. Empty input returns an empty GeometryCollection.
     * @node geometry_envelope @receiver geometry @alias geometryEnvelope
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.envelope(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function envelope(this: geometry, { geometry: geometry }): geometry;

    /**
     * Returns a Polygon's exterior ring as a closed LineString.
     * @node geometry_exterior_ring @receiver geometry @alias geometryExteriorRing
     * @param geometry — Polygon to inspect (receiver: `this` in `x.exteriorRing(...)`)
     * @returns geometryOut — Exterior ring as a LineString
     */
    function exteriorRing(this: geometry<Polygon>, { geometry: geometry<Polygon> }): geometry<LineString>;

    /**
     * Extracts validated geometries and properties from a GeoJSON FeatureCollection. Null Feature geometries are rejected because Geometry values cannot be null.
     * @node geometry_feature_collection_geometries @alias geometryFeatureCollectionGeometries
     * @param featureCollection — GeoJSON FeatureCollection object
     * @returns geometryOut — GeometryCollection containing each Feature Geometry
     * @returns properties — Property objects in Feature order
     */
    function featureCollectionGeometries({ featureCollection: Struct }): { geometryOut: geometry<GeometryCollection>, properties: Struct[] };

    /**
     * Extracts and validates the non-null Geometry and properties from a GeoJSON Feature.
     * @node geometry_feature_geometry @alias geometryFeatureGeometry
     * @param feature — GeoJSON Feature object
     * @returns geometryOut — Extracted Geometry
     * @returns properties — Feature properties, or an empty object when omitted or null
     */
    function featureGeometry({ feature: Struct }): { geometryOut: geometry, properties: Struct };

    /**
     * Parses a GeoJSON geometry object, validates its two-dimensional WGS 84 coordinates and normalizes ring winding. Retains bbox and foreign members. Feature wrappers require extraction.
     * @node geometry_from_geojson @alias geometryFromGeojson
     * @param text — text
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromGeoJson({ text: string }): geometry;

    /**
     * Converts the coordinate vector emitted by H3 Cell Boundary into Geometry. Closes the ring, validates topology, and returns a split MultiPolygon for a transmeridian cell.
     * @node geometry_from_legacy_boundary @alias geometryFromLegacyBoundary
     * @param boundary — boundary
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromLegacyBoundary({ boundary: Struct }): geometry;

    /**
     * Converts the existing GeoCoordinate latitude/longitude object to a Geometry Point without swapping the axes.
     * @node geometry_from_legacy_coordinate @alias geometryFromLegacyCoordinate
     * @param coordinate — coordinate
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromLegacyCoordinate({ coordinate: Struct }): geometry<Point>;

    /**
     * Extracts a Point from a search result or waypoint coordinate. Returns the original rich location wrapper unchanged.
     * @node geometry_from_legacy_location @alias geometryFromLegacyLocation
     * @param location — location
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     * @returns locationOut — Original location wrapper
     */
    function fromLegacyLocation({ location: Struct }): { geometryOut: geometry<Point>, locationOut: Struct };

    /**
     * Converts the existing H3 polygon vector, preserving exterior and interior rings, closing each ring, and splitting transmeridian outlines.
     * @node geometry_from_legacy_polygons @alias geometryFromLegacyPolygons
     * @param polygons — polygons
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromLegacyPolygons({ polygons: Struct }): geometry<MultiPolygon>;

    /**
     * Extracts a LineString from RouteResult.geometry.points or RouteGeometry.points. Returns the original wrapper unchanged.
     * @node geometry_from_legacy_route @alias geometryFromLegacyRoute
     * @param route — route
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     * @returns routeOut — Original route wrapper
     */
    function fromLegacyRoute({ route: Struct }): { geometryOut: geometry<LineString>, routeOut: Struct };

    /**
     * Converts an EPSG:3857 GeoJSON-like Struct in meters to a validated WGS 84 Geometry. All projected positions must be inside the Web Mercator world bounds.
     * @node geometry_from_web_mercator @alias geometryFromWebMercator
     * @param projected — GeoJSON-like Struct marked with crs EPSG:3857
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromWebMercator({ projected: Struct }): geometry;

    /**
     * Parses two-dimensional WKB bytes. Calling this node asserts WGS 84 longitude/latitude. Unsupported dimensions, SRIDs and out-of-range coordinates are rejected.
     * @node geometry_from_wkb @alias geometryFromWkb
     * @param bytes — bytes
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromWkb({ bytes: bytes[] }): geometry;

    /**
     * Parses two-dimensional WKT. Calling this node asserts coordinates are WGS 84 longitude/latitude; it does not transform a projected CRS. Empty scalar geometries are rejected.
     * @node geometry_from_wkt @alias geometryFromWkt
     * @param text — text
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function fromWkt({ text: string }): geometry;

    /**
     * Computes WGS 84 ellipsoidal area in square meters, subtracting polygon holes and summing collection members. Points and lines contribute zero. Each polygon must describe a region smaller than half the Earth.
     * @node geometry_geodesic_area @receiver geometry @alias geometryGeodesicArea
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicArea(...)`)
     * @returns area — Area in square meters
     */
    function geodesicArea(this: geometry, { geometry: geometry }): float;

    /**
     * Returns the initial WGS 84 geodesic bearing from one Point to another in degrees clockwise from north.
     * @node geometry_geodesic_bearing @receiver a @alias geometryGeodesicBearing
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicBearing(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns bearing — Initial bearing in degrees clockwise from north
     */
    function geodesicBearing(this: geometry<Point>, { a: geometry<Point>, b: geometry<Point> }): float;

    /**
     * Approximates a WGS 84 geodesic circle around a Point using 3 through 1024 segments. Antimeridian crossings are split into valid polygon parts, and circles that contain a pole are rejected.
     * @node geometry_geodesic_circle @receiver origin @alias geometryGeodesicCircle
     * @param origin — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicCircle(...)`)
     * @param radius — Positive radius in meters
     * @param segments — Number of polygon segments from 3 through 1024
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function geodesicCircle(this: geometry<Point>, { origin: geometry<Point>, radius: float, segments: int }): geometry;

    /**
     * Returns the WGS 84 destination Point reached from an origin, initial bearing in degrees, and nonnegative distance in meters.
     * @node geometry_geodesic_destination @receiver origin @alias geometryGeodesicDestination
     * @param origin — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicDestination(...)`)
     * @param bearing — bearing
     * @param distance — distance
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function geodesicDestination(this: geometry<Point>, { origin: geometry<Point>, bearing: float, distance: float }): geometry<Point>;

    /**
     * Computes the WGS 84 ellipsoidal geodesic distance between two Points in meters, including antimeridian crossings.
     * @node geometry_geodesic_distance @receiver a @alias geometryGeodesicDistance
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicDistance(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns distance — Distance in meters
     */
    function geodesicDistance(this: geometry<Point>, { a: geometry<Point>, b: geometry<Point> }): float;

    /**
     * Computes discrete Fréchet distance between LineString position sequences using WGS 84 ellipsoidal point distances in meters.
     * @node geometry_geodesic_frechet_distance @receiver a @alias geometryGeodesicFrechetDistance
     * @param a — First LineString (receiver: `this` in `x.geodesicFrechetDistance(...)`)
     * @param b — Second LineString
     * @returns distance — Discrete Fréchet distance in meters
     */
    function geodesicFrechetDistance(this: geometry<LineString>, { a: geometry<LineString>, b: geometry<LineString> }): float;

    /**
     * Returns a Point at a ratio from zero to one along the WGS 84 geodesic between two Points.
     * @node geometry_geodesic_interpolate @receiver a @alias geometryGeodesicInterpolate
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicInterpolate(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @param ratio — ratio
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function geodesicInterpolate(this: geometry<Point>, { a: geometry<Point>, b: geometry<Point>, ratio: float }): geometry<Point>;

    /**
     * Sums WGS 84 ellipsoidal geodesic segment lengths in meters, including polygon exterior and interior ring perimeters. Points contribute zero.
     * @node geometry_geodesic_length @receiver geometry @alias geometryGeodesicLength
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.geodesicLength(...)`)
     * @returns length — Length in meters
     */
    function geodesicLength(this: geometry, { geometry: geometry }): float;

    /**
     * Returns one immediate geometry member by zero-based index. A simple geometry has one member at index zero.
     * @node geometry_n @receiver geometry @alias geometryN
     * @param geometry — Geometry to inspect (receiver: `this` in `x.geometryN(...)`)
     * @param index — Zero-based member index
     * @returns geometryOut — Selected geometry member
     */
    function geometryN(this: geometry, { geometry: geometry, index: int }): geometry;

    /**
     * Returns an H3 cell boundary as a Polygon, or as a split MultiPolygon when the cell crosses the antimeridian.
     * @node geometry_h3_cell_boundary @alias geometryH3CellBoundary
     * @param cell — H3 cell index
     * @returns geometryOut — Cell boundary Polygon or antimeridian-split MultiPolygon
     */
    function h3CellBoundary({ cell: string }): geometry;

    /**
     * Dissolves unique H3 cells at one resolution into a MultiPolygon Geometry.
     * @node geometry_h3_cells_to_geometry @alias geometryH3CellsToGeometry
     * @param cells (optional) — Unique H3 cells at one resolution
     * @returns geometryOut — Dissolved MultiPolygon
     */
    function h3CellsToGeometry({ cells?: string[] }): geometry<MultiPolygon>;

    /**
     * Computes symmetric planar Hausdorff distance in coordinate degrees using only stored coordinate positions. It does not measure distance between continuous segments.
     * @node geometry_hausdorff_distance @receiver a @alias geometryHausdorffDistance
     * @param a — First geometry (receiver: `this` in `x.hausdorffDistance(...)`)
     * @param b — Second geometry
     * @returns distance — Vertex-set Hausdorff distance in degrees
     */
    function hausdorffDistance(this: geometry, { a: geometry, b: geometry }): float;

    /**
     * Returns a Polygon's interior rings as closed LineStrings.
     * @node geometry_interior_rings @receiver geometry @alias geometryInteriorRings
     * @param geometry — Polygon to inspect (receiver: `this` in `x.interiorRings(...)`)
     * @returns rings — Interior rings as LineStrings
     */
    function interiorRings(this: geometry<Polygon>, { geometry: geometry<Polygon> }): geometry<LineString>[];

    /**
     * Returns a Point at a ratio from zero to one along a LineString using planar length.
     * @node geometry_interpolate_line @receiver geometry @alias geometryInterpolateLine
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.interpolateLine(...)`)
     * @param ratio — ratio
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function interpolateLine(this: geometry<LineString>, { geometry: geometry<LineString>, ratio: float }): geometry<Point>;

    /**
     * Constructs the intersection of any Geometry inputs in the longitude/latitude coordinate plane. Preserves point, line and polygon results; mixed results use a GeometryCollection.
     * @node geometry_intersection @receiver a @alias geometryIntersection
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.intersection(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function intersection(this: geometry, { a: geometry, b: geometry }): geometry;

    /**
     * Tests whether geometries share any point in the longitude/latitude coordinate plane, including boundary touches.
     * @node geometry_intersects @receiver a @alias geometryIntersects
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.intersects(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function intersects(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Tests whether the first and last LineString positions are equal.
     * @node geometry_line_is_closed @receiver geometry @alias geometryLineIsClosed
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.isClosed(...)`)
     * @returns result — Predicate result
     */
    function isClosed(this: geometry<LineString>, { geometry: geometry<LineString> }): bool;

    /**
     * Tests whether a multi-geometry or GeometryCollection contains no coordinate positions.
     * @node geometry_is_empty @receiver geometry @alias geometryIsEmpty
     * @param geometry — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.isEmpty(...)`)
     * @returns result — Whether the geometry has no coordinate positions
     */
    function isEmpty(this: geometry, { geometry: geometry }): bool;

    /**
     * Tests whether a LineString is closed and simple, with no crossings, self-touches, or overlapping segments.
     * @node geometry_line_is_ring @receiver geometry @alias geometryLineIsRing
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.isRing(...)`)
     * @returns result — Predicate result
     */
    function isRing(this: geometry<LineString>, { geometry: geometry<LineString> }): bool;

    /**
     * Checks OGC Simple Feature topology after the Geometry value has passed structural validation.
     * @node geometry_is_topologically_valid @receiver geometry @alias geometryIsTopologicallyValid
     * @param geometry — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.isTopologicallyValid(...)`)
     * @returns result — Whether the geometry satisfies OGC topology rules
     */
    function isTopologicallyValid(this: geometry, { geometry: geometry }): bool;

    /**
     * Preserves connections to historical coordinate and route data when a saved board upgrades to Geometry pins.
     * @node geometry_legacy_adapter @alias geometryLegacyAdapter
     * @param mode (optional) — Historical payload conversion
     * @param value — Value to convert
     * @param source (optional) — Original H3 cell or cells used to preserve historical boundaries
     * @returns converted — Converted historical value or Geometry
     */
    function legacyAdapter({ mode?: string, value: Struct, source?: any }): geometry<Point>;

    /**
     * Extracts the part of a LineString between two planar length ratios. Ratios must satisfy 0 <= start < end <= 1.
     * @node geometry_line_substring @receiver geometry @alias geometryLineSubstring
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.lineSubstring(...)`)
     * @param startRatio — start_ratio
     * @param endRatio — end_ratio
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function lineSubstring(this: geometry<LineString>, { geometry: geometry<LineString>, startRatio: float, endRatio: float }): geometry<LineString>;

    /**
     * Returns the planar length ratio of the closest position on a LineString to an input Point.
     * @node geometry_locate_point @receiver geometry @alias geometryLocatePoint
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.locatePoint(...)`)
     * @param point — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns ratio — Planar length ratio from zero to one
     */
    function locatePoint(this: geometry<LineString>, { geometry: geometry<LineString>, point: geometry<Point> }): float;

    /**
     * Creates a rectangular Polygon from west, south, east and north WGS 84 bounds. The envelope must have positive width and height.
     * @node geometry_make_envelope @alias geometryMakeEnvelope
     * @param west — Minimum longitude in degrees
     * @param south — Minimum latitude in degrees
     * @param east — Maximum longitude in degrees
     * @param north — Maximum latitude in degrees
     * @returns geometryOut — Rectangular Polygon
     */
    function makeEnvelope({ west: float, south: float, east: float, north: float }): geometry<Polygon>;

    /**
     * Wraps a Geometry and properties in a GeoJSON Feature. A non-empty id is included as the Feature id.
     * @node geometry_make_feature @receiver geometry @alias geometryMakeFeature
     * @param geometry — Feature Geometry (receiver: `this` in `x.makeFeature(...)`)
     * @param properties (optional) — Feature properties
     * @param id (optional) — Optional string Feature id
     * @returns feature — GeoJSON Feature object
     */
    function makeFeature(this: geometry, { geometry: geometry, properties?: Struct, id?: string }): Struct;

    /**
     * Creates a GeoJSON FeatureCollection from Geometry values and an optional matching array of property objects.
     * @node geometry_make_feature_collection @receiver geometries @alias geometryMakeFeatureCollection
     * @param geometries (optional) — Feature geometries (receiver: `this` in `x.makeFeatureCollection(...)`)
     * @param properties (optional) — Property objects, either empty or one per Geometry
     * @returns featureCollection — GeoJSON FeatureCollection object
     */
    function makeFeatureCollection(this: geometry[], { geometries?: geometry[], properties?: Struct[] }): Struct;

    /**
     * Creates a GeometryCollection from any validated geometries, including nested collections.
     * @node geometry_make_geometry_collection @receiver geometries @alias geometryMakeGeometryCollection
     * @param geometries (optional) — Member geometries (receiver: `this` in `x.makeGeometryCollection(...)`)
     * @returns geometryOut — Constructed GeometryCollection
     */
    function makeGeometryCollection(this: geometry[], { geometries?: geometry[] }): geometry<GeometryCollection>;

    /**
     * Creates a LineString from an ordered array of Point geometries. At least two Points are required.
     * @node geometry_make_line_string @receiver points @alias geometryMakeLineString
     * @param points — Ordered Point geometries (receiver: `this` in `x.makeLineString(...)`)
     * @returns geometryOut — Constructed LineString
     */
    function makeLineString(this: geometry<Point>[], { points: geometry<Point>[] }): geometry<LineString>;

    /**
     * Creates a MultiLineString from LineString geometries. An empty input creates an empty MultiLineString.
     * @node geometry_make_multi_line_string @receiver lines @alias geometryMakeMultiLineString
     * @param lines (optional) — LineString geometries (receiver: `this` in `x.makeMultiLineString(...)`)
     * @returns geometryOut — Constructed MultiLineString
     */
    function makeMultiLineString(this: geometry<LineString>[], { lines?: geometry<LineString>[] }): geometry<MultiLineString>;

    /**
     * Creates a MultiPoint from Point geometries. An empty input creates an empty MultiPoint.
     * @node geometry_make_multi_point @receiver points @alias geometryMakeMultiPoint
     * @param points (optional) — Point geometries (receiver: `this` in `x.makeMultiPoint(...)`)
     * @returns geometryOut — Constructed MultiPoint
     */
    function makeMultiPoint(this: geometry<Point>[], { points?: geometry<Point>[] }): geometry<MultiPoint>;

    /**
     * Creates a MultiPolygon from Polygon geometries. An empty input creates an empty MultiPolygon.
     * @node geometry_make_multi_polygon @receiver polygons @alias geometryMakeMultiPolygon
     * @param polygons (optional) — Polygon geometries (receiver: `this` in `x.makeMultiPolygon(...)`)
     * @returns geometryOut — Constructed MultiPolygon
     */
    function makeMultiPolygon(this: geometry<Polygon>[], { polygons?: geometry<Polygon>[] }): geometry<MultiPolygon>;

    /**
     * Creates a WGS 84 Point from longitude and latitude. Both coordinates must be finite and within geographic bounds.
     * @node geometry_make_point @alias geometryMakePoint
     * @param longitude — longitude
     * @param latitude — latitude
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function makePoint({ longitude: float, latitude: float }): geometry<Point>;

    /**
     * Creates a Polygon from an exterior LineString and optional interior LineString rings. Open rings are closed automatically.
     * @node geometry_make_polygon @receiver exterior @alias geometryMakePolygon
     * @param exterior — Exterior ring as a LineString (receiver: `this` in `x.makePolygon(...)`)
     * @param holes (optional) — Interior rings as LineStrings
     * @returns geometryOut — Constructed Polygon
     */
    function makePolygon(this: geometry<LineString>, { exterior: geometry<LineString>, holes?: geometry<LineString>[] }): geometry<Polygon>;

    /**
     * Merges MultiLineString members at exactly equal endpoints. Chains stop at endpoints with a degree other than two, preserving network junctions.
     * @node geometry_merge_connected_lines @receiver geometry @alias geometryMergeConnectedLines
     * @param geometry — MultiLineString to merge (receiver: `this` in `x.mergeConnectedLines(...)`)
     * @returns geometryOut — Merged line chains
     */
    function mergeConnectedLines(this: geometry<MultiLineString>, { geometry: geometry<MultiLineString> }): geometry<MultiLineString>;

    /**
     * Returns the smallest-area rotated planar rectangle containing all positions. Degenerate inputs return a Point or LineString.
     * @node geometry_minimum_rotated_rectangle @receiver geometry @alias geometryMinimumRotatedRectangle
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.minimumRotatedRectangle(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function minimumRotatedRectangle(this: geometry, { geometry: geometry }): geometry;

    /**
     * Counts immediate members of a multi-geometry or GeometryCollection. A simple geometry has one member.
     * @node geometry_num_geometries @receiver geometry @alias geometryNumGeometries
     * @param geometry — Geometry to inspect (receiver: `this` in `x.numGeometries(...)`)
     * @returns count — Number of immediate members
     */
    function numGeometries(this: geometry, { geometry: geometry }): int;

    /**
     * Counts a Polygon's interior rings.
     * @node geometry_num_interior_rings @receiver geometry @alias geometryNumInteriorRings
     * @param geometry — Polygon to inspect (receiver: `this` in `x.numInteriorRings(...)`)
     * @returns count — Number of interior rings
     */
    function numInteriorRings(this: geometry<Polygon>, { geometry: geometry<Polygon> }): int;

    /**
     * Counts coordinate positions, including closing polygon positions and recursive collection members.
     * @node geometry_num_points @receiver geometry @alias geometryNumPoints
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.numPoints(...)`)
     * @returns count — Number of positions, including closing ring positions
     */
    function numPoints(this: geometry, { geometry: geometry }): int;

    /**
     * Normalizes Polygon or MultiPolygon winding to counterclockwise exterior rings and clockwise interior rings, then validates topology.
     * @node geometry_orient_polygon_rings @receiver geometry @alias geometryOrientPolygonRings
     * @param geometry — Polygon or MultiPolygon to orient (receiver: `this` in `x.orientPolygonRings(...)`)
     * @returns geometryOut — Oriented Polygon or MultiPolygon
     */
    function orientPolygonRings(this: geometry, { geometry: geometry }): geometry;

    /**
     * Tests whether same-dimensional inputs partly overlap without either covering the other.
     * @node geometry_overlaps @receiver a @alias geometryOverlaps
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.overlaps(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function overlaps(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Explodes a multi-geometry or GeometryCollection into its immediate members. A simple geometry returns a one-item array.
     * @node geometry_parts @receiver geometry @alias geometryParts
     * @param geometry — Geometry to explode (receiver: `this` in `x.parts(...)`)
     * @returns parts — Immediate geometry members
     */
    function parts(this: geometry, { geometry: geometry }): geometry[];

    /**
     * Computes area in square coordinate degrees, subtracting polygon holes and summing collection members. Points and lines contribute zero.
     * @node geometry_planar_area @receiver geometry @alias geometryPlanarArea
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.planarArea(...)`)
     * @returns area — Planar area in square coordinate degrees
     */
    function planarArea(this: geometry, { geometry: geometry }): float;

    /**
     * Buffers a geometry in coordinate degrees. The operation is planar, does not wrap at the antimeridian, and rejects results outside WGS 84 coordinate bounds.
     * @node geometry_planar_buffer @receiver geometry @alias geometryPlanarBuffer
     * @param geometry — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.planarBuffer(...)`)
     * @param distance (optional) — Signed buffer distance in coordinate degrees. Negative distances shrink polygonal inputs.
     * @param cap (optional) — Line endpoint style: Round, Square, or Butt
     * @param join (optional) — Corner style: Round, Miter, or Bevel
     * @param arcStepDegrees (optional) — Maximum angular step used to approximate round caps and joins
     * @param miterMinAngleDegrees (optional) — Minimum corner angle that retains a miter instead of beveling it
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function planarBuffer(this: geometry, { geometry: geometry, distance?: float, cap?: string, join?: string, arcStepDegrees?: float, miterMinAngleDegrees?: float }): geometry<MultiPolygon>;

    /**
     * Computes the shortest planar distance in coordinate degrees. This longitude/latitude plane does not wrap at the antimeridian. Empty inputs are rejected.
     * @node geometry_planar_distance @receiver a @alias geometryPlanarDistance
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.planarDistance(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns distance — Planar distance in coordinate degrees
     */
    function planarDistance(this: geometry, { a: geometry, b: geometry }): float;

    /**
     * Computes discrete Fréchet distance between LineString position sequences using planar coordinate degrees.
     * @node geometry_planar_frechet_distance @receiver a @alias geometryPlanarFrechetDistance
     * @param a — First LineString (receiver: `this` in `x.planarFrechetDistance(...)`)
     * @param b — Second LineString
     * @returns distance — Discrete Fréchet distance in coordinate degrees
     */
    function planarFrechetDistance(this: geometry<LineString>, { a: geometry<LineString>, b: geometry<LineString> }): float;

    /**
     * Sums line lengths and polygon ring perimeters in planar coordinate degrees. Points contribute zero.
     * @node geometry_planar_length @receiver geometry @alias geometryPlanarLength
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.planarLength(...)`)
     * @returns length — Planar length in coordinate degrees
     */
    function planarLength(this: geometry, { geometry: geometry }): float;

    /**
     * Returns one coordinate position by zero-based traversal index, including polygon ring closing positions.
     * @node geometry_point_n @receiver geometry @alias geometryPointN
     * @param geometry — Geometry to inspect (receiver: `this` in `x.pointN(...)`)
     * @param index — Zero-based coordinate position index
     * @returns geometryOut — Selected coordinate Point
     */
    function pointN(this: geometry, { geometry: geometry, index: int }): geometry<Point>;

    /**
     * Returns a representative point that intersects the input and lies in its interior when possible. Empty geometry has no point on surface.
     * @node geometry_point_on_surface @receiver geometry @alias geometryPointOnSurface
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.pointOnSurface(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function pointOnSurface(this: geometry, { geometry: geometry }): geometry<Point>;

    /**
     * Converts a Geometry Point to an H3 cell index at resolution 0 through 15.
     * @node geometry_point_to_h3_cell @receiver geometry @alias geometryPointToH3Cell
     * @param geometry — Point to index (receiver: `this` in `x.pointToH3Cell(...)`)
     * @param resolution (optional) — H3 resolution from 0 through 15
     * @returns cell — H3 cell index
     */
    function pointToH3Cell(this: geometry<Point>, { geometry: geometry<Point>, resolution?: int }): string;

    /**
     * Returns every coordinate position as a Point in traversal order, including polygon ring closing positions.
     * @node geometry_points @receiver geometry @alias geometryPoints
     * @param geometry — Geometry to inspect (receiver: `this` in `x.points(...)`)
     * @returns points — Coordinate positions as Points
     */
    function points(this: geometry, { geometry: geometry }): geometry<Point>[];

    /**
     * Covers a Polygon or MultiPolygon with H3 cells. Longitude edges use the Geometry contract's direct interpolation, so antimeridian regions must already be split. The containment mode controls whether centroids, complete boundaries or intersections qualify.
     * @node geometry_polygon_to_h3_cells @receiver geometry @alias geometryPolygonToH3Cells
     * @param geometry — Polygon or MultiPolygon to cover (receiver: `this` in `x.polygonToH3Cells(...)`)
     * @param resolution (optional) — H3 resolution from 0 through 15
     * @param containment (optional) — One of centroid, contains-boundary, intersects-boundary or covers
     * @param maxCells (optional) — Maximum cells to emit and basis for the preflight work budget, from 1 through 1000000
     * @returns cells — H3 cell indexes
     */
    function polygonToH3Cells(this: geometry, { geometry: geometry, resolution?: int, containment?: string, maxCells?: int }): string[];

    /**
     * Tests a DE-9IM relation pattern. The pattern has nine characters chosen from T, F, *, 0, 1, and 2.
     * @node geometry_relate_pattern @receiver a @alias geometryRelatePattern
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.relatePattern(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @param pattern (optional) — Nine-character DE-9IM pattern using T, F, *, 0, 1, and 2
     * @returns result — Whether the relation matches the pattern
     */
    function relatePattern(this: geometry, { a: geometry, b: geometry, pattern?: string }): bool;

    /**
     * Removes consecutive duplicate line and ring positions and duplicate MultiPoint members, then validates the result.
     * @node geometry_remove_repeated_points @receiver geometry @alias geometryRemoveRepeatedPoints
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.removeRepeatedPoints(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function removeRepeatedPoints(this: geometry, { geometry: geometry }): geometry;

    /**
     * Repairs a limited set of topology defects: consecutive duplicate coordinates, duplicate MultiPoint members, overlapping MultiPolygon parts, and polygon self-intersections that planar overlay can resolve. GeometryCollection members are repaired independently. Other defects return an error.
     * @node geometry_repair @receiver geometry @alias geometryRepair
     * @param geometry — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.repair(...)`)
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function repair(this: geometry, { geometry: geometry }): geometry;

    /**
     * Returns a LineString with its positions in reverse order.
     * @node geometry_reverse_line @receiver geometry @alias geometryReverseLine
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.reverseLine(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function reverseLine(this: geometry<LineString>, { geometry: geometry<LineString> }): geometry<LineString>;

    /**
     * Rotates geometry counterclockwise in the coordinate plane around the center of its bounding box.
     * @node geometry_rotate @receiver geometry @alias geometryRotate
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.rotate(...)`)
     * @param degrees — degrees
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function rotate(this: geometry, { geometry: geometry, degrees: float }): geometry;

    /**
     * Scales geometry in the coordinate plane around the center of its bounding box. Scale factors must be nonzero.
     * @node geometry_scale @receiver geometry @alias geometryScale
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.scale(...)`)
     * @param xFactor — x_factor
     * @param yFactor — y_factor
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function scale(this: geometry, { geometry: geometry, xFactor: float, yFactor: float }): geometry;

    /**
     * Returns the shortest connection from A to B in coordinate degrees. Intersecting inputs return a Point; disjoint inputs return a LineString.
     * @node geometry_shortest_line @receiver a @alias geometryShortestLine
     * @param a — Source geometry (receiver: `this` in `x.shortestLine(...)`)
     * @param b — Destination geometry
     * @returns geometryOut — Shortest Point or LineString connection from A to B
     */
    function shortestLine(this: geometry, { a: geometry, b: geometry }): geometry;

    /**
     * Simplifies lines and polygon rings with a nonnegative tolerance in coordinate degrees. Validates the result and rejects a topology-breaking result.
     * @node geometry_simplify @receiver geometry @alias geometrySimplify
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.simplify(...)`)
     * @param tolerance — tolerance
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function simplify(this: geometry, { geometry: geometry, tolerance: float }): geometry;

    /**
     * Simplifies line and polygon vertices using planar triangle areas in square coordinate degrees. The result is validated before it is returned.
     * @node geometry_simplify_vw @receiver geometry @alias geometrySimplifyVw
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.simplifyVw(...)`)
     * @param epsilon — epsilon
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function simplifyVw(this: geometry, { geometry: geometry, epsilon: float }): geometry;

    /**
     * Uses the topology-aware Visvalingam-Whyatt variant, then validates the complete result. Epsilon is measured in square coordinate degrees.
     * @node geometry_simplify_vw_preserve @receiver geometry @alias geometrySimplifyVwPreserve
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.simplifyVwPreserve(...)`)
     * @param epsilon — epsilon
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function simplifyVwPreserve(this: geometry, { geometry: geometry, epsilon: float }): geometry;

    /**
     * Skews geometry by x and y angles in the coordinate plane around the center of its bounding box.
     * @node geometry_skew @receiver geometry @alias geometrySkew
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.skew(...)`)
     * @param xDegrees — x_degrees
     * @param yDegrees — y_degrees
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function skew(this: geometry, { geometry: geometry, xDegrees: float, yDegrees: float }): geometry;

    /**
     * Applies Chaikin corner cutting to line and polygon geometry. Each iteration approximately doubles its position count.
     * @node geometry_smooth_chaikin @receiver geometry @alias geometrySmoothChaikin
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.smoothChaikin(...)`)
     * @param iterations — Number of smoothing iterations
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function smoothChaikin(this: geometry, { geometry: geometry, iterations: int }): geometry;

    /**
     * Rounds every longitude and latitude to the nearest multiple of a positive grid size in degrees. Fails if coordinates leave WGS 84 bounds or topology collapses.
     * @node geometry_snap_to_grid @receiver geometry @alias geometrySnapToGrid
     * @param geometry — Geometry to snap (receiver: `this` in `x.snapToGrid(...)`)
     * @param gridSize — Positive longitude and latitude grid spacing in degrees
     * @returns geometryOut — Snapped geometry
     */
    function snapToGrid(this: geometry, { geometry: geometry, gridSize: float }): geometry;

    /**
     * Splits LineString segments whose longitude jump exceeds 180 degrees and returns a MultiLineString with paired endpoints at longitude 180 and -180.
     * @node geometry_split_line_antimeridian @receiver geometry @alias geometrySplitLineAntimeridian
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.splitLineAntimeridian(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function splitLineAntimeridian(this: geometry<LineString>, { geometry: geometry<LineString> }): geometry<MultiLineString>;

    /**
     * Projects a Point to the nearest planar position on a LineString and splits there when the distance is within the nonnegative tolerance in degrees. Endpoint splits return the original line as one member.
     * @node geometry_split_line_at_point @receiver geometry @alias geometrySplitLineAtPoint
     * @param geometry — LineString to split (receiver: `this` in `x.splitLineAtPoint(...)`)
     * @param point — Point to project onto the line
     * @param tolerance (optional) — Maximum planar projection distance in degrees
     * @returns geometryOut — One or two split line members
     */
    function splitLineAtPoint(this: geometry<LineString>, { geometry: geometry<LineString>, point: geometry<Point>, tolerance?: float }): geometry<MultiLineString>;

    /**
     * Returns the first Point of a LineString.
     * @node geometry_start_point @receiver geometry @alias geometryStartPoint
     * @param geometry — LineString to inspect (receiver: `this` in `x.startPoint(...)`)
     * @returns geometryOut — First Point
     */
    function startPoint(this: geometry<LineString>, { geometry: geometry<LineString> }): geometry<Point>;

    /**
     * Returns Polygon or MultiPolygon regions belonging to exactly one input. Returns a MultiPolygon.
     * @node geometry_symmetric_difference @receiver a @alias geometrySymmetricDifference
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.symmetricDifference(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function symmetricDifference(this: geometry, { a: geometry, b: geometry }): geometry<MultiPolygon>;

    /**
     * Writes a validated geometry as GeoJSON text, retaining bbox and foreign members and normalizing ring winding.
     * @node geometry_to_geojson @receiver geometry @alias geometryToGeojson
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.toGeoJson(...)`)
     * @returns text — Serialized geometry
     */
    function toGeoJson(this: geometry, { geometry: geometry }): string;

    /**
     * Converts a Geometry Point into the existing GeoCoordinate shape used by H3, routing, search and map nodes.
     * @node geometry_to_legacy_coordinate @receiver geometry @alias geometryToLegacyCoordinate
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.toLegacyCoordinate(...)`)
     * @returns coordinate — Legacy latitude/longitude object
     */
    function toLegacyCoordinate(this: geometry<Point>, { geometry: geometry<Point> }): Struct;

    /**
     * Updates only the points of an existing RouteResult or RouteGeometry with a LineString, retaining route metadata and other fields.
     * @node geometry_to_legacy_route @alias geometryToLegacyRoute
     * @param route — route
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns routeOut — Route with updated points and retained metadata
     */
    function toLegacyRoute({ route: Struct, geometry: geometry<LineString> }): Struct;

    /**
     * Wraps a Point, LineString or Polygon in its corresponding multi-geometry. Existing multi-geometries pass through unchanged.
     * @node geometry_to_multi @receiver geometry @alias geometryToMulti
     * @param geometry — Input geometry (receiver: `this` in `x.toMulti(...)`)
     * @returns geometryOut — Result geometry
     */
    function toMulti(this: geometry, { geometry: geometry }): geometry;

    /**
     * Unwraps a multi-geometry containing exactly one member. Simple geometries pass through unchanged.
     * @node geometry_to_single @receiver geometry @alias geometryToSingle
     * @param geometry — Input geometry (receiver: `this` in `x.toSingle(...)`)
     * @returns geometryOut — Result geometry
     */
    function toSingle(this: geometry, { geometry: geometry }): geometry;

    /**
     * Projects WGS 84 positions to EPSG:3857 meters. Latitude must be within the Web Mercator bound of +/-85.0511287798066 degrees; positions nearer a pole have no EPSG:3857 representation and are rejected. The result is a Struct because Geometry values always contain WGS 84 longitude and latitude.
     * @node geometry_to_web_mercator @receiver geometry @alias geometryToWebMercator
     * @param geometry — Validated WGS 84 geometry with latitude within +/-85.0511 degrees (receiver: `this` in `x.toWebMercator(...)`)
     * @returns projected — GeoJSON-like EPSG:3857 geometry with coordinates in meters
     */
    function toWebMercator(this: geometry, { geometry: geometry }): Struct;

    /**
     * Writes two-dimensional WKB bytes. WKB omits GeoJSON bbox and foreign members; keep application properties in a surrounding Struct.
     * @node geometry_to_wkb @receiver geometry @alias geometryToWkb
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.toWkb(...)`)
     * @returns bytes — WKB byte sequence
     */
    function toWkb(this: geometry, { geometry: geometry }): bytes[];

    /**
     * Writes two-dimensional WKT. WKT omits GeoJSON bbox and foreign members; keep application properties in a surrounding Struct.
     * @node geometry_to_wkt @receiver geometry @alias geometryToWkt
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.toWkt(...)`)
     * @returns text — Serialized geometry
     */
    function toWkt(this: geometry, { geometry: geometry }): string;

    /**
     * Tests whether both inputs describe the same point set using DE-9IM semantics. Coordinate order may differ.
     * @node geometry_topologically_equals @receiver a @alias geometryTopologicallyEquals
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.topologicallyEquals(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function topologicallyEquals(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Tests whether the inputs share boundary points but have disjoint interiors using DE-9IM semantics.
     * @node geometry_touches @receiver a @alias geometryTouches
     * @param a — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.touches(...)`)
     * @param b — Validated two-dimensional WGS 84 GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function touches(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Offsets every position by planar longitude and latitude degrees. Results outside WGS 84 coordinate bounds fail validation.
     * @node geometry_translate @receiver geometry @alias geometryTranslate
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.translate(...)`)
     * @param longitudeOffset — longitude_offset
     * @param latitudeOffset — latitude_offset
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function translate(this: geometry, { geometry: geometry, longitudeOffset: float, latitudeOffset: float }): geometry;

    /**
     * Triangulates Polygon or MultiPolygon input with an ear-cutting algorithm and returns a GeometryCollection of triangle Polygons.
     * @node geometry_triangulate @receiver geometry @alias geometryTriangulate
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.triangulate(...)`)
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function triangulate(this: geometry, { geometry: geometry }): geometry<GeometryCollection>;

    /**
     * Returns the GeoJSON geometry type name.
     * @node geometry_type @receiver geometry @alias geometryType
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.type(...)`)
     * @returns type — GeoJSON type name
     */
    function type(this: geometry, { geometry: geometry }): string;

    /**
     * Efficiently dissolves an array of Polygon and MultiPolygon values. An empty array returns an empty MultiPolygon.
     * @node geometry_unary_union @receiver geometries @alias geometryUnaryUnion
     * @param geometries (optional) — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.unaryUnion(...)`)
     * @returns geometryOut — Validated two-dimensional WGS 84 GeoJSON geometry
     */
    function unaryUnion(this: geometry[], { geometries?: geometry[] }): geometry<MultiPolygon>;

    /**
     * Returns up to 256 topology errors. If more exist, the final array entry reports truncation. A valid geometry returns an empty array.
     * @node geometry_validation_errors @receiver geometry @alias geometryValidationErrors
     * @param geometry — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.validationErrors(...)`)
     * @returns errors — Topology validation errors, capped at 256 entries plus a truncation notice
     */
    function validationErrors(this: geometry, { geometry: geometry }): string[];

    /**
     * Returns the first OGC topology error, or an empty string when the geometry is topologically valid.
     * @node geometry_validity_reason @receiver geometry @alias geometryValidityReason
     * @param geometry — Validated two-dimensional WGS 84 GeoJSON geometry (receiver: `this` in `x.validityReason(...)`)
     * @returns reason — First topology error, or an empty string
     */
    function validityReason(this: geometry, { geometry: geometry }): string;

    /**
     * Tests whether geometry B contains A in the longitude/latitude coordinate plane.
     * @node geometry_within @receiver a @alias geometryWithin
     * @param a — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.within(...)`)
     * @param b — Validated WGS 84 longitude/latitude GeoJSON geometry
     * @returns result — Planar predicate result
     */
    function within(this: geometry, { a: geometry, b: geometry }): bool;

    /**
     * Converts a GeoJSON-like Point, MultiPoint, or point-only GeometryCollection Struct with arbitrary finite longitudes into Geometry. Longitudes are wrapped to the half-open interval [-180, 180), while latitudes must already be within [-90, 90]. Connected geometries are rejected because wrapping individual vertices would tear them across the antimeridian.
     * @node geometry_wrap_longitude @alias geometryWrapLongitude
     * @param geometry — GeoJSON-like Point, MultiPoint, or point-only GeometryCollection Struct with finite longitudes and WGS 84 latitudes
     * @returns geometryOut — Validated WGS 84 GeoJSON geometry
     */
    function wrapLongitude({ geometry: Struct }): geometry;

    /**
     * Returns the Point x coordinate, longitude in degrees.
     * @node geometry_x @receiver geometry @alias geometryX
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.x(...)`)
     * @returns value — Coordinate in degrees
     */
    function x(this: geometry<Point>, { geometry: geometry<Point> }): float;

    /**
     * Returns the Point y coordinate, latitude in degrees.
     * @node geometry_y @receiver geometry @alias geometryY
     * @param geometry — Validated WGS 84 longitude/latitude GeoJSON geometry (receiver: `this` in `x.y(...)`)
     * @returns value — Coordinate in degrees
     */
    function y(this: geometry<Point>, { geometry: geometry<Point> }): float;
}

declare namespace h3 {
    // === Web/Geo/H3 ===

    /**
     * Calculates the area of an H3 cell in the specified unit.
     * @node h3_cell_area @alias h3CellArea
     * @param cell (optional) — H3 cell index
     * @param unit (optional) — Area unit for the result
     * @returns area — Area of the cell in the specified unit
     * @returns resolution — Resolution of the cell
     */
    function cellArea({ cell?: string, unit?: Struct }): { area: float, resolution: int };

    /**
     * Returns the polygon boundary (vertices) of an H3 cell. Useful for visualization and geospatial operations.
     * @node h3_cell_to_boundary @alias h3CellToBoundary
     * @param cell (optional) — H3 cell index as a hexadecimal string
     * @returns geometryOut — Cell boundary as a Polygon, or MultiPolygon when it crosses the antimeridian
     * @returns vertexCount — Number of vertices (typically 6 for hexagons, 5 for pentagons)
     */
    function cellToBoundary({ cell?: string }): { geometryOut: geometry, vertexCount: int };

    /**
     * Returns all child cells at a finer resolution that fit within the given cell.
     * @node h3_cell_to_children @alias h3CellToChildren
     * @param cell (optional) — H3 cell index
     * @param childResolution (optional) — Target resolution for children (must be higher than cell's resolution)
     * @returns children — Array of child H3 cell indices
     * @returns count — Number of child cells
     */
    function cellToChildren({ cell?: string, childResolution?: int }): { children: string[], count: int };

    /**
     * Converts an H3 cell index to the geographic coordinate of its center point.
     * @node h3_cell_to_latlng @alias h3CellToLatlng
     * @param cell (optional) — H3 cell index as a hexadecimal string
     * @returns geometryOut — Point at the center of the H3 cell
     */
    function cellToLatlng({ cell?: string }): geometry<Point>;

    /**
     * Returns the parent cell at a coarser resolution. The parent contains the given cell.
     * @node h3_cell_to_parent @alias h3CellToParent
     * @param cell (optional) — H3 cell index
     * @param parentResolution (optional) — Target resolution for the parent (must be lower than cell's resolution)
     * @returns parent — Parent H3 cell index at the specified resolution
     * @returns originalResolution — Resolution of the input cell
     */
    function cellToParent({ cell?: string, parentResolution?: int }): { parent: string, originalResolution: int };

    /**
     * Converts a set of H3 cells to polygon boundaries. Returns the outline(s) of the cell set, merging adjacent cells.
     * @node h3_cells_to_multi_polygon @alias h3CellsToMultiPolygon
     * @param cells (optional) — Array of H3 cell indices
     * @returns geometryOut — Merged cell boundaries as a MultiPolygon, split at the antimeridian
     * @returns polygonCount — Number of polygons in Geometry, including pieces split at the antimeridian
     */
    function cellsToMultiPolygon({ cells?: string[] }): { geometryOut: geometry<MultiPolygon>, polygonCount: int };

    /**
     * Compacts a set of H3 cells by replacing groups of cells with their parent when all children are present. Reduces the number of cells while covering the same area.
     * @node h3_compact_cells @alias h3CompactCells
     * @param cells (optional) — Array of H3 cell indices to compact
     * @returns compacted — Array of compacted H3 cell indices (may contain mixed resolutions)
     * @returns originalCount — Number of input cells
     * @returns compactedCount — Number of cells after compaction
     */
    function compactCells({ cells?: string[] }): { compacted: string[], originalCount: int, compactedCount: int };

    /**
     * Returns the average edge length of H3 cells at a given resolution.
     * @node h3_edge_length @alias h3EdgeLength
     * @param resolution (optional) — H3 resolution (0-15)
     * @param unit (optional) — Length unit for the result
     * @returns edgeLength — Average edge length at this resolution
     * @returns cellCount — Total number of cells at this resolution covering Earth
     */
    function edgeLength({ resolution?: int, unit?: Struct }): { edgeLength: float, cellCount: int };

    /**
     * Returns all H3 cells within k steps of the origin cell (a filled disk of hexagons). Useful for proximity searches and area coverage.
     * @node h3_grid_disk @alias h3GridDisk
     * @param cell (optional) — Origin H3 cell index
     * @param k (optional) — Number of rings around the origin (0 = just the origin cell)
     * @returns cells — Array of H3 cell indices in the disk
     * @returns count — Number of cells in the disk
     */
    function gridDisk({ cell?: string, k?: int }): { cells: string[], count: int };

    /**
     * Calculates the grid distance (number of steps) between two H3 cells. Both cells must be at the same resolution.
     * @node h3_grid_distance @alias h3GridDistance
     * @param cellA (optional) — First H3 cell index
     * @param cellB (optional) — Second H3 cell index
     * @returns distance — Grid distance (number of hexagon steps) between the cells
     */
    function gridDistance({ cellA?: string, cellB?: string }): int;

    /**
     * Finds a path of H3 cells between two cells. Returns all cells along the shortest path. Both cells must be at the same resolution.
     * @node h3_grid_path @alias h3GridPath
     * @param cellA (optional) — Starting H3 cell index
     * @param cellB (optional) — Ending H3 cell index
     * @returns path — Array of H3 cell indices along the path (including start and end)
     * @returns length — Number of cells in the path
     */
    function gridPath({ cellA?: string, cellB?: string }): { path: string[], length: int };

    /**
     * Converts a geographic coordinate to an H3 cell index at the specified resolution. H3 is a hierarchical hexagonal grid system.
     * @node h3_latlng_to_cell @alias h3LatlngToCell
     * @param geometry — Point to index as an H3 cell
     * @param resolution (optional) — H3 resolution (0-15). Higher = smaller cells. 0 = ~4,357,449 km², 15 = ~0.9 m²
     * @returns cell — H3 cell index as a hexadecimal string
     */
    function latlngToCell({ geometry: geometry<Point>, resolution?: int }): string;
}

declare namespace http {
    // === Web ===

    /**
     * Downloads a file from a url
     * @node http_download @receiver request @alias httpDownload
     * @param request — The HTTP request to perform (receiver: `this` in `x.download(...)`)
     * @param flowPath — The path to save the file to
     * @impure has side effects / drives control flow
     */
    function download(this: HttpRequest, { request: Struct, flowPath: Struct }): void;

    // === Web/API ===

    /**
     * Performs an HTTP request
     * @node http_fetch @receiver request @alias httpFetch
     * @param request — The HTTP request to perform (receiver: `this` in `x.fetch(...)`)
     * @returns response — The HTTP response
     * @impure has side effects / drives control flow
     */
    function fetch(this: HttpRequest, { request: Struct }): Struct;

    /**
     * Performs an HTTP request
     * @node streaming_http_fetch @receiver request @alias streamingHttpFetch
     * @param request — The HTTP request to perform (receiver: `this` in `x.fetchStreaming(...)`)
     * @returns streamingResponse — The HTTP response
     * @returns response — The HTTP response
     * @impure has side effects / drives control flow
     */
    function fetchStreaming(this: HttpRequest, { request: Struct }): { streamingResponse: bytes[], response: Struct };

    // === Web/API/Request ===

    /**
     * Gets a header from a http request
     * @node http_get_header @receiver request @alias httpGetHeader
     * @param request — The http request (receiver: `this` in `x.getHeader(...)`)
     * @param header — The header to get
     * @returns found — True if the header was found
     * @returns value — The value of the header
     */
    function getHeader(this: HttpRequest, { request: Struct, header: string }): { found: bool, value: string };

    /**
     * Gets all headers from a http request
     * @node http_get_headers @receiver request @alias httpGetHeaders
     * @param request — The http request (receiver: `this` in `x.getHeaders(...)`)
     * @returns headers — The headers of the request
     */
    function getHeaders(this: HttpRequest, { request: Struct }): Map<string, string>;

    /**
     * Gets the method from a http request
     * @node http_get_method @receiver request @alias httpGetMethod
     * @param request — The http request (receiver: `this` in `x.getMethod(...)`)
     * @returns method — The method of the request
     */
    function getMethod(this: HttpRequest, { request: Struct }): string;

    /**
     * Gets the url from a http request
     * @node http_get_url @receiver request @alias httpGetUrl
     * @param request — The http request (receiver: `this` in `x.getUrl(...)`)
     * @returns url — The url of the request
     */
    function getUrl(this: HttpRequest, { request: Struct }): string;

    /**
     * Creates a http request
     * @node http_make_request @alias httpMakeRequest
     * @param method (optional) — Http Method GET,POST etc.
     * @param url — The request URL
     * @returns request — The http request
     */
    function request({ method?: string, url: string }): Struct;

    /**
     * Sets the Accept header of a http request
     * @node http_set_accept @receiver request @alias httpSetAccept
     * @param request — The http request (receiver: `this` in `x.setAccept(...)`)
     * @param accept (optional) — The accept header value
     * @returns requestOut — The http request
     */
    function setAccept(this: HttpRequest, { request: Struct, accept?: string }): Struct;

    /**
     * Sets the Authorization header using a Bearer token
     * @node http_set_bearer_auth @receiver request @alias httpSetBearerAuth
     * @param request — The http request (receiver: `this` in `x.setBearerAuth(...)`)
     * @param token — Bearer token
     * @returns requestOut — The http request
     */
    function setBearerAuth(this: HttpRequest, { request: Struct, token: string }): Struct;

    /**
     * Sets the body of a http request
     * @node http_set_bytes_body @receiver request @alias httpSetBytesBody
     * @param request — The http request (receiver: `this` in `x.setBytesBody(...)`)
     * @param body — The body of the request
     * @returns requestOut — The http request
     */
    function setBytesBody(this: HttpRequest, { request: Struct, body: bytes[] }): Struct;

    /**
     * Sets the Content-Type header of a http request
     * @node http_set_content_type @receiver request @alias httpSetContentType
     * @param request — The http request (receiver: `this` in `x.setContentType(...)`)
     * @param contentType (optional) — The content type value
     * @returns requestOut — The http request
     */
    function setContentType(this: HttpRequest, { request: Struct, contentType?: string }): Struct;

    /**
     * Sets the body of a http request to form-encoded data
     * @node http_set_form_body @receiver request @alias httpSetFormBody
     * @param request — The http request (receiver: `this` in `x.setFormBody(...)`)
     * @param fields (optional) — Form fields to encode
     * @param setContentType (optional) — Adds application/x-www-form-urlencoded when missing
     * @returns requestOut — The http request
     */
    function setFormBody(this: HttpRequest, { request: Struct, fields?: Struct, setContentType?: bool }): Struct;

    /**
     * Sets a header of a http request
     * @node http_set_header @receiver request @alias httpSetHeader
     * @param request — The http request (receiver: `this` in `x.setHeader(...)`)
     * @param name — The name of the header
     * @param value — The value of the header
     * @returns requestOut — The http request
     */
    function setHeader(this: HttpRequest, { request: Struct, name: string, value: string }): Struct;

    /**
     * Sets the headers of a http request
     * @node http_set_headers @receiver request @alias httpSetHeaders
     * @param request — The http request (receiver: `this` in `x.setHeaders(...)`)
     * @param headers — The headers of the request
     * @param merge (optional) — Merge with existing headers instead of replacing
     * @returns requestOut — The http request
     */
    function setHeaders(this: HttpRequest, { request: Struct, headers: Map<string, string>, merge?: bool }): Struct;

    /**
     * Sets the method of a http request
     * @node http_set_method @receiver request @alias httpSetMethod
     * @param request — The http request (receiver: `this` in `x.setMethod(...)`)
     * @param method (optional) — The method of the request
     * @returns requestOut — The http request
     */
    function setMethod(this: HttpRequest, { request: Struct, method?: string }): Struct;

    /**
     * Sets the body of a http request
     * @node http_set_string_body @receiver request @alias httpSetStringBody
     * @param request — The http request (receiver: `this` in `x.setStringBody(...)`)
     * @param body — The body of the request
     * @returns requestOut — The http request
     */
    function setStringBody(this: HttpRequest, { request: Struct, body: string }): Struct;

    /**
     * Sets the body of a http request
     * @node http_set_struct_body @receiver request @alias httpSetStructBody
     * @param request — The http request (receiver: `this` in `x.setStructBody(...)`)
     * @param body — The body of the request
     * @returns requestOut — The http request
     */
    function setStructBody(this: HttpRequest, { request: Struct, body: Struct }): Struct;

    /**
     * Sets the url of a http request
     * @node http_set_url @receiver request @alias httpSetUrl
     * @param request — The http request (receiver: `this` in `x.setUrl(...)`)
     * @param url — The url of the request
     * @returns requestOut — The http request
     */
    function setUrl(this: HttpRequest, { request: Struct, url: string }): Struct;

    // === Web/API/Response ===

    /**
     * Gets a header from a http request
     * @node http_response_get_header @receiver response @alias httpResponseGetHeader
     * @param response — The http response (receiver: `this` in `x.header(...)`)
     * @param header — The header to get
     * @returns found — True if the header was found
     * @returns value — The value of the header
     */
    function header(this: HttpResponse, { response: Struct, header: string }): { found: bool, value: string };

    /**
     * Gets all headers from a http request
     * @node http_response_get_headers @receiver response @alias httpResponseGetHeaders
     * @param response — The http response (receiver: `this` in `x.headers(...)`)
     * @returns headers — The headers of the response
     */
    function headers(this: HttpResponse, { response: Struct }): Map<string, string>;

    /**
     * Checks if the status code of a http response is a success
     * @node http_response_is_success @receiver response @alias httpResponseIsSuccess
     * @param response — The http response (receiver: `this` in `x.isSuccess(...)`)
     * @returns isSuccess — True if the status code is a success
     */
    function isSuccess(this: HttpResponse, { response: Struct }): bool;

    /**
     * Gets the status code from a http response
     * @node http_response_get_status @receiver response @alias httpResponseGetStatus
     * @param response — The http response (receiver: `this` in `x.status(...)`)
     * @returns statusCode — The status code of the response
     */
    function status(this: HttpResponse, { response: Struct }): int;

    /**
     * Gets the body of a http response as bytes
     * @node http_response_to_bytes @receiver response @alias httpResponseToBytes
     * @param response — The http response (receiver: `this` in `x.toBytes(...)`)
     * @returns bytes — The body of the response as bytes
     * @impure has side effects / drives control flow
     */
    function toBytes(this: HttpResponse, { response: Struct }): bytes[];

    /**
     * Gets the body of a http response as json
     * @node http_response_to_json @receiver response @alias httpResponseToJson
     * @param response — The http response (receiver: `this` in `x.toJson(...)`)
     * @returns struct — The body of the response as json
     * @impure has side effects / drives control flow
     */
    function toJson(this: HttpResponse, { response: Struct }): Struct;

    /**
     * Gets the body of a http response as text
     * @node http_response_to_text @receiver response @alias httpResponseToText
     * @param response — The http response (receiver: `this` in `x.toText(...)`)
     * @returns text — The body of the response as text
     * @impure has side effects / drives control flow
     */
    function toText(this: HttpResponse, { response: Struct }): string;
}

declare namespace image {
    // === Web/Camera ===

    /**
     * Writes an image to a data URL
     * @node image_write_dataurl @receiver image @alias imageWriteDataurl
     * @param image — The image to write to a data URL (receiver: `this` in `x.toDataUrl(...)`)
     * @param format (optional) — The format of the image (e.g., png, jpeg)
     * @returns url — The data URL of the written image
     * @impure has side effects / drives control flow
     */
    function toDataUrl(this: NodeImage, { image: Struct, format?: string }): string;
}

declare namespace mcp {
    // === Web/MCP ===

    /**
     * Registers MCP server authentication settings.
     * @node mcp_register_auth @receiver config_in @alias mcpRegisterAuth
     * @param configIn — MCP server config (receiver: `this` in `x.registerAuth(...)`)
     * @param auth — Auth config
     * @param flowLikeAuth (optional) — Hosted public OAuth servers only: resolve the verified caller to a Flow-Like account and enforce app permissions. Requires a trusted platform issuer and an allowed OAuth client.
     * @returns configOut — Updated config
     */
    function registerAuth(this: McpServerConfig, { configIn: Struct, auth: Struct, flowLikeAuth?: bool }): Struct;

    /**
     * Registers referenced Flow functions as MCP tools.
     * @node mcp_register_functions @receiver config_in @alias mcpRegisterFunctions
     * @param configIn — MCP server config (receiver: `this` in `x.registerFunctions(...)`)
     * @returns configOut — Updated config
     */
    function registerFunctions(this: McpServerConfig, { configIn: Struct }): Struct;

    /**
     * Registers a static MCP prompt template.
     * @node mcp_register_prompt @receiver config_in @alias mcpRegisterPrompt
     * @param configIn — MCP server config (receiver: `this` in `x.registerPrompt(...)`)
     * @param name — Prompt name
     * @param description — Optional description
     * @param template — Prompt template
     * @returns configOut — Updated config
     */
    function registerPrompt(this: McpServerConfig, { configIn: Struct, name: string, description: string, template: string }): Struct;

    /**
     * Registers a FlowPath as an MCP resource.
     * @node mcp_register_resource @receiver config_in @alias mcpRegisterResource
     * @param configIn — MCP server config (receiver: `this` in `x.registerResource(...)`)
     * @param flowPath — Resource FlowPath
     * @param uri — MCP resource URI exposed to clients. Defaults to file://<flow path> when empty.
     * @param name — Resource display name. Defaults to the FlowPath filename when empty.
     * @param description — Optional description
     * @returns configOut — Updated config
     */
    function registerResource(this: McpServerConfig, { configIn: Struct, flowPath: Struct, uri: string, name: string, description: string }): Struct;

    /**
     * Starts an MCP server from a composed config.
     * @node mcp_server @alias mcpServer
     * @param config — MCP server config
     * @returns localAddr — Bound address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): string;

    /**
     * Creates an MCP server config that function, resource, prompt, auth, and server nodes can compose.
     * @node mcp_server_config @alias mcpServerConfig
     * @param host (optional) — Bind host
     * @param port (optional) — Bind port
     * @param path (optional) — MCP HTTP path
     * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
     * @param maxConnections (optional) — Maximum concurrent requests
     * @param maxBodyBytes (optional) — Maximum HTTP request body size
     * @param tls — TLS security config
     * @returns config — MCP server config
     */
    function serverConfig({ host?: string, port?: int, path?: string, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;
}

declare namespace mqtt {
    // === Web/MQTT ===

    /**
     * Binds a lightweight MQTT broker for daemon workflows. Typed lifecycle events are exposed as pins; published messages are delivered to the referenced on-message handler.
     * @node mqtt_broker @alias mqttBroker
     * @param config — MQTT broker configuration
     * @returns localAddr — Bound broker socket address
     * @returns clientId — Connected MQTT client id
     * @returns remoteAddr — Remote client socket address
     * @impure has side effects / drives control flow
     */
    function broker({ config: Struct }): { localAddr: string, clientId: string, remoteAddr: string };

    /**
     * Connects to an MQTT broker and returns a session reference for use with Publish, Subscribe, and Disconnect nodes.
     * @node mqtt_connect @alias mqttConnect
     * @param config — MQTT connection configuration (host, port, client_id, optional credentials, TLS)
     * @returns session — MQTT session reference for use with Publish/Subscribe/Disconnect nodes
     * @impure has side effects / drives control flow
     */
    function connect({ config: Struct }): Struct;

    /**
     * Disconnects from an MQTT broker and cleans up the session
     * @node mqtt_disconnect @receiver session @alias mqttDisconnect
     * @param session — MQTT session to disconnect (receiver: `this` in `x.disconnect(...)`)
     * @impure has side effects / drives control flow
     */
    function disconnect(this: MqttSession, { session: Struct }): void;

    /**
     * Publishes a message to an MQTT topic
     * @node mqtt_publish @receiver session @alias mqttPublish
     * @param session — MQTT session reference (receiver: `this` in `x.publish(...)`)
     * @param topic — The MQTT topic to publish to
     * @param payload — The message content to publish
     * @param qos (optional) — Quality of Service level
     * @param retain (optional) — Whether the broker should retain this message
     * @impure has side effects / drives control flow
     */
    function publish(this: MqttSession, { session: Struct, topic: string, payload: string, qos?: string, retain?: bool }): void;

    /**
     * Subscribes to an MQTT topic and invokes a handler for each incoming message. Holds execution until the connection closes or timeout, then triggers on_close.
     * @node mqtt_subscribe @receiver session @alias mqttSubscribe
     * @param session — MQTT session reference (receiver: `this` in `x.subscribe(...)`)
     * @param topic — The MQTT topic filter to subscribe to
     * @param qos (optional) — Quality of Service level for the subscription
     * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
     * @impure has side effects / drives control flow
     */
    function subscribe(this: MqttSession, { session: Struct, topic: string, qos?: string, timeoutSeconds?: int }): void;
}

declare namespace rest {
    // === Web/REST ===

    /**
     * Registers REST server authentication settings.
     * @node rest_register_auth @receiver config_in @alias restRegisterAuth
     * @param configIn — REST server config (receiver: `this` in `x.registerAuth(...)`)
     * @param auth (optional) — Auth config
     * @returns configOut — Updated config
     */
    function registerAuth(this: RestServerConfig, { configIn: Struct, auth?: Struct }): Struct;

    /**
     * Registers a FlowPath file or directory as static REST responses.
     * @node rest_register_files @receiver config_in @alias restRegisterFiles
     * @param configIn — REST server config (receiver: `this` in `x.registerFiles(...)`)
     * @param path — HTTP route path
     * @param flowPath — File or directory FlowPath
     * @param directory (optional) — Serve the FlowPath as a directory mount
     * @param contentType (optional) — Optional response content type override
     * @returns configOut — Updated config
     */
    function registerFiles(this: RestServerConfig, { configIn: Struct, path: string, flowPath: Struct, directory?: bool, contentType?: string }): Struct;

    /**
     * Registers referenced Flow functions as handlers for a REST path.
     * @node rest_register_function @receiver config_in @alias restRegisterFunction
     * @param configIn — REST server config (receiver: `this` in `x.registerFunction(...)`)
     * @param path — HTTP route path
     * @param method (optional) — Allowed HTTP method. ANY accepts all methods.
     * @returns configOut — Updated config
     */
    function registerFunction(this: RestServerConfig, { configIn: Struct, path: string, method?: string }): Struct;

    /**
     * Registers OpenAPI JSON and browser UI endpoints generated from the REST server config.
     * @node rest_register_open_api @receiver config_in @alias restRegisterOpenApi
     * @param configIn — REST server config (receiver: `this` in `x.registerOpenApi(...)`)
     * @param path (optional) — OpenAPI JSON route path
     * @param uiPath (optional) — OpenAPI browser UI route path; empty disables the UI
     * @returns configOut — Updated config
     */
    function registerOpenApi(this: RestServerConfig, { configIn: Struct, path?: string, uiPath?: string }): Struct;

    /**
     * Starts a REST server from a composed config. Function routes and files are registered on the config before this node runs.
     * @node rest_server @alias restServer
     * @param config — REST server config
     * @returns localAddr — Bound address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): string;

    /**
     * Creates a REST server config that route, file, auth, and server nodes can compose.
     * @node rest_server_config @alias restServerConfig
     * @param host (optional) — Bind host
     * @param port (optional) — Bind port
     * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
     * @param maxConnections (optional) — Maximum concurrent requests
     * @param maxBodyBytes (optional) — Maximum HTTP request body size
     * @param tls — TLS security config
     * @returns config — REST server config
     */
    function serverConfig({ host?: string, port?: int, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;
}

declare namespace tcp {
    // === Web/TCP ===

    /**
     * Closes an open TCP connection gracefully
     * @node tcp_close @receiver session @alias tcpClose
     * @param session — TCP session to close (receiver: `this` in `x.close(...)`)
     * @impure has side effects / drives control flow
     */
    function close(this: TcpSession, { session: Struct }): void;

    /**
     * Opens a TCP connection to a remote host. Triggers on_connect with the session, then invokes the on-message handler for each incoming data chunk. Holds execution until the connection closes, then triggers on_close.
     * @node tcp_connect @alias tcpConnect
     * @param config — TCP connection configuration (host, port, optional timeout)
     * @returns session — TCP session reference for use with Send/Close nodes
     * @impure has side effects / drives control flow
     */
    function connect({ config: Struct }): Struct;

    /**
     * Binds a TCP listener on a port. Fires on_listening, then accepts incoming connections and invokes the handler for each. Holds execution until closed or timed out, then triggers on_close.
     * @node tcp_listen @alias tcpListen
     * @param config — TCP listener configuration (host, port, optional timeout, max connections)
     * @impure has side effects / drives control flow
     */
    function listen({ config: Struct }): void;

    /**
     * Sends data through an open TCP connection
     * @node tcp_send @receiver session @alias tcpSend
     * @param session — TCP session reference (receiver: `this` in `x.send(...)`)
     * @param messageType (optional) — Whether to send as text (UTF-8) or binary
     * @param payload — The data to send (string for Text, byte array for Binary)
     * @impure has side effects / drives control flow
     */
    function send(this: TcpSession, { session: Struct, messageType?: string, payload: string }): void;

    /**
     * Binds a TCP server. Typed lifecycle events are exposed as pins; incoming data chunks are delivered to the referenced on-message handler.
     * @node tcp_server @alias tcpServer
     * @param config — TCP server configuration
     * @returns localAddr — Bound local socket address
     * @returns session — Accepted TCP client session
     * @returns remoteAddr — Remote client socket address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };
}

declare namespace tls {
    // === Web/TLS ===

    /**
     * Creates a local certificate authority certificate and private key.
     * @node create_ca_certificate @alias createCaCertificate
     * @param commonName (optional) — Certificate authority common name
     * @returns certificate — Certificate authority PEM bundle
     * @impure has side effects / drives control flow
     */
    function createCaCertificate({ commonName?: string }): Struct;

    /**
     * Creates a server or client certificate signed by a local certificate authority.
     * @node create_ca_signed_certificate @alias createCaSignedCertificate
     * @param ca — Certificate authority PEM bundle
     * @param commonName (optional) — Certificate common name
     * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
     * @param usage (optional) — Certificate usage
     * @returns certificate — Signed certificate PEM bundle
     * @impure has side effects / drives control flow
     */
    function createCaSignedCertificate({ ca: Struct, commonName?: string, subjectAltNames?: string[], usage?: string }): Struct;

    /**
     * Creates a self-signed certificate and private key.
     * @node create_self_signed_certificate @alias createSelfSignedCertificate
     * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
     * @returns certificate — Self-signed certificate PEM bundle
     * @impure has side effects / drives control flow
     */
    function createSelfSignedCertificate({ subjectAltNames?: string[] }): Struct;
}

declare namespace udp {
    // === Web/UDP ===

    /**
     * Binds a UDP socket to a local address and port
     * @node udp_bind @alias udpBind
     * @param config — UDP bind configuration (host and port)
     * @returns session — UDP session reference for use with SendTo/Receive/Close nodes
     * @impure has side effects / drives control flow
     */
    function bind({ config: Struct }): Struct;

    /**
     * Closes a bound UDP socket and releases resources
     * @node udp_close @receiver session @alias udpClose
     * @param session — UDP session to close (receiver: `this` in `x.close(...)`)
     * @impure has side effects / drives control flow
     */
    function close(this: UdpSession, { session: Struct }): void;

    /**
     * Listens for incoming datagrams on a bound UDP socket. Invokes the on-message handler for each received datagram. Holds execution until the socket is closed or the timeout expires, then fires on_close.
     * @node udp_receive @receiver session @alias udpReceive
     * @param session — UDP session reference from a Bind node (receiver: `this` in `x.receive(...)`)
     * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
     * @impure has side effects / drives control flow
     */
    function receive(this: UdpSession, { session: Struct, timeoutSeconds?: int }): void;

    /**
     * Sends a datagram to a target address through a bound UDP socket
     * @node udp_send_to @receiver session @alias udpSendTo
     * @param session — UDP session reference (receiver: `this` in `x.sendTo(...)`)
     * @param targetHost — Destination host address
     * @param targetPort — Destination port number
     * @param payload — The message content to send
     * @returns bytesSent — Number of bytes sent
     * @impure has side effects / drives control flow
     */
    function sendTo(this: UdpSession, { session: Struct, targetHost: string, targetPort: int, payload: string }): int;

    /**
     * Binds a UDP socket. Typed lifecycle pins describe the socket; incoming datagrams are delivered to the referenced on-message handler.
     * @node udp_server @alias udpServer
     * @param config — UDP server configuration
     * @returns session — UDP server socket session
     * @returns localAddr — Bound local socket address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): { session: Struct, localAddr: string };
}

declare namespace web {
    // === Web/Scraping ===

    /**
     * Extracts links from the input text
     * @node web_scrape_extract_links @alias webScrapeExtractLinks
     * @param startingPage — The page to start extracting links from
     * @param sameDomain (optional) — Stay on the same domain or subdomains
     * @param offsetMs (optional) — Delay between requests
     * @param depth (optional) — The depth to extract links from
     * @returns links — The extracted links
     * @impure has side effects / drives control flow
     */
    function extractLinks({ startingPage: string, sameDomain?: bool, offsetMs?: int, depth?: int }): Set<string>;
}

declare namespace websocket {
    // === Web/WebSocket ===

    /**
     * Closes an open WebSocket connection gracefully
     * @node websocket_close @receiver session @alias websocketClose
     * @param session — WebSocket session to close (receiver: `this` in `x.close(...)`)
     * @impure has side effects / drives control flow
     */
    function close(this: WebSocketSession, { session: Struct }): void;

    /**
     * Opens a WebSocket connection. Immediately triggers on_connect with the session, then invokes on_message for each incoming message. Holds execution until the connection closes, then triggers on_close.
     * @node websocket_connect @alias websocketConnect
     * @param config — WebSocket connection configuration (URL, optional headers, optional timeout)
     * @returns session — WebSocket session reference for use with Send/Close nodes
     * @impure has side effects / drives control flow
     */
    function connect({ config: Struct }): Struct;

    /**
     * Sends a message through an open WebSocket connection
     * @node websocket_send @receiver session @alias websocketSend
     * @param session — WebSocket session reference (receiver: `this` in `x.send(...)`)
     * @param messageType (optional) — Whether to send as text or binary
     * @param payload — The message content to send (string for Text, byte array for Binary)
     * @impure has side effects / drives control flow
     */
    function send(this: WebSocketSession, { session: Struct, messageType?: string, payload: string }): void;

    /**
     * Binds a WebSocket server. Typed lifecycle events are exposed as pins; incoming messages are delivered to the referenced on-message handler.
     * @node websocket_server @alias websocketServer
     * @param config — WebSocket server configuration
     * @returns localAddr — Bound local socket address
     * @returns session — Accepted WebSocket client session
     * @returns remoteAddr — Remote client socket address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };
}
