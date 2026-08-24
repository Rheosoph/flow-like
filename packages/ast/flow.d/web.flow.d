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
    // === Web/Geo/Map ===

    /**
     * Fetches a static map image for the given coordinates using OpenStreetMap tiles. Returns a satellite/standard map image centered on the location.
     * @node geo_get_map_image @alias geoGetMapImage
     * @param coordinate — The geographic coordinate (latitude, longitude) to center the map on
     * @param zoom (optional) — Map zoom level (1-19). Higher values show more detail. Default: 15
     * @param width (optional) — Image width in pixels. Default: 512
     * @param height (optional) — Image height in pixels. Default: 512
     * @param style (optional) — Map style to use
     * @returns image — The fetched map image
     * @impure has side effects / drives control flow
     */
    function getMapImage({ coordinate: Struct, zoom?: int, width?: int, height?: int, style?: string }): Struct;

    // === Web/Geo/Routing ===

    /**
     * Snaps noisy GPS traces to the road network using OSRM map matching.
     * @node geo_osrm_match_trace @alias geoOsrmMatchTrace
     * @param coordinates (optional) — Ordered GPS coordinates to match
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param timestamps (optional) — Optional UNIX timestamps for each coordinate (seconds)
     * @param radiuses (optional) — Optional search radiuses in meters for each coordinate
     * @param gaps (optional) — How to handle gaps: split or ignore
     * @param tidy (optional) — Simplify the matched geometry
     * @param baseUrl (optional) — OSRM server base URL
     * @returns matchings — Matched routes for the trace
     * @returns primaryMatching — Primary matched route
     * @returns tracepoints — Tracepoints mapped to the road network
     * @impure has side effects / drives control flow
     */
    function osrmMatchTrace({ coordinates?: Struct[], profile?: Struct, timestamps?: int[], radiuses?: float[], gaps?: string, tidy?: bool, baseUrl?: string }): { matchings: Struct[], primaryMatching: Struct, tracepoints: Struct[] };

    /**
     * Finds the nearest routable point(s) to a coordinate using OSRM.
     * @node geo_osrm_nearest @alias geoOsrmNearest
     * @param coordinate — The coordinate to snap to the road network
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param number (optional) — Maximum number of nearest points to return (1-50)
     * @param baseUrl (optional) — OSRM server base URL
     * @returns nearest — The closest routable point
     * @returns waypoints — List of nearest routable points
     * @impure has side effects / drives control flow
     */
    function osrmNearest({ coordinate: Struct, profile?: Struct, number?: int, baseUrl?: string }): { nearest: Struct, waypoints: Struct[] };

    /**
     * Computes travel time and distance matrices between coordinates using OSRM.
     * @node geo_osrm_table @alias geoOsrmTable
     * @param coordinates (optional) — List of coordinates to include in the matrix
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
    function osrmTable({ coordinates?: Struct[], profile?: string, sources?: int[], destinations?: int[], includeDurations?: bool, includeDistances?: bool, baseUrl?: string }): { durations: Struct[], distances: Struct[], result: Struct };

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
     * @param coordinates (optional) — Ordered coordinates for the trip
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
     * @returns geometry — Trip geometry as array of coordinates
     * @impure has side effects / drives control flow
     */
    function osrmTrip({ coordinates?: Struct[], profile?: Struct, roundtrip?: bool, source?: string, destination?: string, baseUrl?: string }): { trip: Struct, trips: Struct[], waypoints: Struct[], distance: float, duration: float, geometry: Struct[] };

    /**
     * Plans a route between two points using the OSRM routing service. Returns turn-by-turn directions, distance, and duration.
     * @node geo_plan_route @alias geoPlanRoute
     * @param start — Starting coordinate for the route
     * @param end — Ending coordinate for the route
     * @param waypoints (optional) — Optional intermediate waypoints to pass through
     * @param profile (optional) — Transportation mode: Car, Bike, or Foot
     * @param alternatives (optional) — Request alternative routes
     * @returns route — The primary calculated route
     * @returns alternativesOut — Alternative routes if requested
     * @returns distance — Total route distance in meters
     * @returns duration — Estimated travel time in seconds
     * @returns geometry — Route geometry as array of coordinates
     * @impure has side effects / drives control flow
     */
    function planRoute({ start: Struct, end: Struct, waypoints?: Struct, profile?: string, alternatives?: bool }): { route: Struct, alternativesOut: Struct, distance: float, duration: float, geometry: Struct };

    // === Web/Geo/Search ===

    /**
     * Converts geographic coordinates to a human-readable address using the Nominatim service (OpenStreetMap).
     * @node geo_reverse_geocode @alias geoReverseGeocode
     * @param coordinate — The geographic coordinate (latitude, longitude) to look up
     * @param zoom (optional) — Level of detail for the address (0-18). Higher = more specific. Default: 18
     * @returns result — The reverse geocoding result with address details
     * @returns displayName — The full formatted address string
     * @impure has side effects / drives control flow
     */
    function reverseGeocode({ coordinate: Struct, zoom?: int }): { result: Struct, displayName: string };

    /**
     * Searches for a location by name or address using the Nominatim geocoding service (OpenStreetMap). Returns matching locations with coordinates.
     * @node geo_search_location @alias geoSearchLocation
     * @param query (optional) — The search query (address, place name, etc.)
     * @param limit (optional) — Maximum number of results to return. Default: 5
     * @param countryCodes (optional) — Optional comma-separated list of country codes to limit search (e.g., 'de,at,ch')
     * @returns results — Array of search results with coordinates
     * @returns firstResult — The first/best matching result (if any)
     * @impure has side effects / drives control flow
     */
    function searchLocation({ query?: string, limit?: int, countryCodes?: string }): { results: Struct[], firstResult: Struct };
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
     * @returns boundary — Array of coordinates representing the cell boundary (closed polygon)
     * @returns vertexCount — Number of vertices (typically 6 for hexagons, 5 for pentagons)
     */
    function cellToBoundary({ cell?: string }): { boundary: Struct, vertexCount: int };

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
     * @returns coordinate — The center coordinate of the H3 cell
     */
    function cellToLatlng({ cell?: string }): Struct;

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
     * @returns polygons — Array of polygons representing the merged cell boundaries
     * @returns polygonCount — Number of separate polygons (disconnected regions)
     */
    function cellsToMultiPolygon({ cells?: string[] }): { polygons: Struct, polygonCount: int };

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
     * @param coordinate — The geographic coordinate (latitude, longitude)
     * @param resolution (optional) — H3 resolution (0-15). Higher = smaller cells. 0 = ~4,357,449 km², 15 = ~0.9 m²
     * @returns cell — H3 cell index as a hexadecimal string
     */
    function latlngToCell({ coordinate: Struct, resolution?: int }): string;
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
     * @returns configOut — Updated config
     */
    function registerAuth(this: McpServerConfig, { configIn: Struct, auth: Struct }): Struct;

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
