// Web — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Web ===

/**
 * Downloads a file from a url
 * @param request — The HTTP request to perform
 * @param flowPath — The path to save the file to
 * @impure has side effects / drives control flow
 */
declare function httpDownload({ request: Struct, flowPath: Struct }): void;


// === Web/API ===

/**
 * Performs an HTTP request
 * @param request — The HTTP request to perform
 * @returns response — The HTTP response
 * @impure has side effects / drives control flow
 */
declare function httpFetch({ request: Struct }): Struct;

/**
 * Performs an HTTP request
 * @param request — The HTTP request to perform
 * @returns streamingResponse — The HTTP response
 * @returns response — The HTTP response
 * @impure has side effects / drives control flow
 */
declare function streamingHttpFetch({ request: Struct }): { streamingResponse: bytes[], response: Struct };


// === Web/API/Request ===

/**
 * Gets a header from a http request
 * @param request — The http request
 * @param header — The header to get
 * @returns found — True if the header was found
 * @returns value — The value of the header
 */
declare function httpGetHeader({ request: Struct, header: string }): { found: bool, value: string };

/**
 * Gets all headers from a http request
 * @param request — The http request
 * @returns headers — The headers of the request
 */
declare function httpGetHeaders({ request: Struct }): Map<string, string>;

/**
 * Gets the method from a http request
 * @param request — The http request
 * @returns method — The method of the request
 */
declare function httpGetMethod({ request: Struct }): string;

/**
 * Gets the url from a http request
 * @param request — The http request
 * @returns url — The url of the request
 */
declare function httpGetUrl({ request: Struct }): string;

/**
 * Creates a http request
 * @param method (optional) — Http Method GET,POST etc.
 * @param url — The request URL
 * @returns request — The http request
 */
declare function httpMakeRequest({ method?: string, url: string }): Struct;

/**
 * Sets the Accept header of a http request
 * @param request — The http request
 * @param accept (optional) — The accept header value
 * @returns requestOut — The http request
 */
declare function httpSetAccept({ request: Struct, accept?: string }): Struct;

/**
 * Sets the Authorization header using a Bearer token
 * @param request — The http request
 * @param token — Bearer token
 * @returns requestOut — The http request
 */
declare function httpSetBearerAuth({ request: Struct, token: string }): Struct;

/**
 * Sets the body of a http request
 * @param request — The http request
 * @param body — The body of the request
 * @returns requestOut — The http request
 */
declare function httpSetBytesBody({ request: Struct, body: bytes[] }): Struct;

/**
 * Sets the Content-Type header of a http request
 * @param request — The http request
 * @param contentType (optional) — The content type value
 * @returns requestOut — The http request
 */
declare function httpSetContentType({ request: Struct, contentType?: string }): Struct;

/**
 * Sets the body of a http request to form-encoded data
 * @param request — The http request
 * @param fields (optional) — Form fields to encode
 * @param setContentType (optional) — Adds application/x-www-form-urlencoded when missing
 * @returns requestOut — The http request
 */
declare function httpSetFormBody({ request: Struct, fields?: Struct, setContentType?: bool }): Struct;

/**
 * Sets a header of a http request
 * @param request — The http request
 * @param name — The name of the header
 * @param value — The value of the header
 * @returns requestOut — The http request
 */
declare function httpSetHeader({ request: Struct, name: string, value: string }): Struct;

/**
 * Sets the headers of a http request
 * @param request — The http request
 * @param headers — The headers of the request
 * @param merge (optional) — Merge with existing headers instead of replacing
 * @returns requestOut — The http request
 */
declare function httpSetHeaders({ request: Struct, headers: Map<string, string>, merge?: bool }): Struct;

/**
 * Sets the method of a http request
 * @param request — The http request
 * @param method (optional) — The method of the request
 * @returns requestOut — The http request
 */
declare function httpSetMethod({ request: Struct, method?: string }): Struct;

/**
 * Sets the body of a http request
 * @param request — The http request
 * @param body — The body of the request
 * @returns requestOut — The http request
 */
declare function httpSetStringBody({ request: Struct, body: string }): Struct;

/**
 * Sets the body of a http request
 * @param request — The http request
 * @param body — The body of the request
 * @returns requestOut — The http request
 */
declare function httpSetStructBody({ request: Struct, body: Struct }): Struct;

/**
 * Sets the url of a http request
 * @param request — The http request
 * @param url — The url of the request
 * @returns requestOut — The http request
 */
declare function httpSetUrl({ request: Struct, url: string }): Struct;


// === Web/API/Response ===

/**
 * Gets a header from a http request
 * @param response — The http response
 * @param header — The header to get
 * @returns found — True if the header was found
 * @returns value — The value of the header
 */
declare function httpResponseGetHeader({ response: Struct, header: string }): { found: bool, value: string };

/**
 * Gets all headers from a http request
 * @param response — The http response
 * @returns headers — The headers of the response
 */
declare function httpResponseGetHeaders({ response: Struct }): Map<string, string>;

/**
 * Gets the status code from a http response
 * @param response — The http response
 * @returns statusCode — The status code of the response
 */
declare function httpResponseGetStatus({ response: Struct }): int;

/**
 * Checks if the status code of a http response is a success
 * @param response — The http response
 * @returns isSuccess — True if the status code is a success
 */
declare function httpResponseIsSuccess({ response: Struct }): bool;

/**
 * Gets the body of a http response as bytes
 * @param response — The http response
 * @returns bytes — The body of the response as bytes
 * @impure has side effects / drives control flow
 */
declare function httpResponseToBytes({ response: Struct }): bytes[];

/**
 * Gets the body of a http response as json
 * @param response — The http response
 * @returns struct — The body of the response as json
 * @impure has side effects / drives control flow
 */
declare function httpResponseToJson({ response: Struct }): Struct;

/**
 * Gets the body of a http response as text
 * @param response — The http response
 * @returns text — The body of the response as text
 * @impure has side effects / drives control flow
 */
declare function httpResponseToText({ response: Struct }): string;


// === Web/Auth ===

/**
 * Creates REST auth that requires a configured API key header.
 * @param header (optional) — Header that carries the API key
 * @param key — Expected API key
 * @returns auth — API key auth config
 */
declare function apiKeyAuth({ header?: string, key: string }): Struct;

/**
 * Creates REST auth that requires HTTP Basic credentials.
 * @param username — Expected username
 * @param password — Expected password
 * @returns auth — Basic auth config
 */
declare function basicAuth({ username: string, password: string }): Struct;

/**
 * Creates REST auth that requires a static Authorization bearer token.
 * @param token — Expected bearer token
 * @returns auth — Bearer token auth config
 */
declare function bearerTokenAuth({ token: string }): Struct;

/**
 * Creates REST auth that verifies an HMAC-SHA256 request signature.
 * @param secret — Shared HMAC secret
 * @param signatureHeader (optional) — Header that carries the lowercase hex HMAC signature
 * @param timestampHeader (optional) — Header that carries the Unix timestamp in seconds
 * @param maxSkewSeconds (optional) — Allowed timestamp skew in seconds; zero disables timestamp freshness checks
 * @returns auth — HMAC auth config
 */
declare function hmacSha256Auth({ secret: string, signatureHeader?: string, timestampHeader?: string, maxSkewSeconds?: int }): Struct;

/**
 * Creates OAuth bearer auth from a JWKS JSON FlowPath loaded when the server starts.
 * @param jwksFlowPath — JWKS JSON file FlowPath
 * @param issuer — Required token issuer. Empty disables issuer validation.
 * @param audience — Required token audience. Empty disables audience validation.
 * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
 * @returns auth — OAuth auth config
 */
declare function oauthJwksFileAuth({ jwksFlowPath: Struct, issuer: string, audience: string, requiredScopes: string[] }): Struct;

/**
 * Creates OAuth bearer auth that fetches a JWKS endpoint once when the server starts.
 * @param jwksUrl — JWKS endpoint URL
 * @param issuer — Required token issuer. Empty disables issuer validation.
 * @param audience — Required token audience. Empty disables audience validation.
 * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
 * @returns auth — OAuth auth config
 */
declare function oauthJwksUrlAuth({ jwksUrl: string, issuer: string, audience: string, requiredScopes: string[] }): Struct;

/**
 * Creates OAuth bearer auth by discovering the JWKS URI from an OpenID Connect issuer.
 * @param issuer — OIDC issuer URL. The server fetches /.well-known/openid-configuration.
 * @param audience — Required token audience. Empty disables audience validation.
 * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
 * @returns auth — OIDC auth config
 */
declare function oidcDiscoveryAuth({ issuer: string, audience: string, requiredScopes: string[] }): Struct;


// === Web/Camera ===

/**
 * Writes an image to a data URL
 * @param image — The image to write to a data URL
 * @param format (optional) — The format of the image (e.g., png, jpeg)
 * @returns url — The data URL of the written image
 * @impure has side effects / drives control flow
 */
declare function imageWriteDataurl({ image: Struct, format?: string }): string;

/**
 * Captures a frame from an IP camera
 * @param request — The HTTP request to perform
 * @returns image — The captured image frame
 * @impure has side effects / drives control flow
 */
declare function webCameraGrabFrame({ request: Struct }): Struct;

/**
 * Captures one frame from an RTSP camera stream
 * @param rtspUrl — RTSP or RTSPS stream URL
 * @param transport (optional) — RTSP RTP transport protocol
 * @param timeoutMs (optional) — Maximum time in milliseconds to connect and decode a frame
 * @param maxFrames (optional) — Maximum video frames to inspect before failing
 * @returns image — The captured RTSP frame
 * @returns errorMessage — Readable capture error
 * @impure has side effects / drives control flow
 */
declare function webCameraGrabRtspFrame({ rtspUrl: string, transport?: string, timeoutMs?: int, maxFrames?: int }): { image: Struct, errorMessage: string };


// === Web/Geo/H3 ===

/**
 * Calculates the area of an H3 cell in the specified unit.
 * @param cell (optional) — H3 cell index
 * @param unit (optional) — Area unit for the result
 * @returns area — Area of the cell in the specified unit
 * @returns resolution — Resolution of the cell
 */
declare function h3CellArea({ cell?: string, unit?: Struct }): { area: float, resolution: int };

/**
 * Returns the polygon boundary (vertices) of an H3 cell. Useful for visualization and geospatial operations.
 * @param cell (optional) — H3 cell index as a hexadecimal string
 * @returns boundary — Array of coordinates representing the cell boundary (closed polygon)
 * @returns vertexCount — Number of vertices (typically 6 for hexagons, 5 for pentagons)
 */
declare function h3CellToBoundary({ cell?: string }): { boundary: Struct, vertexCount: int };

/**
 * Returns all child cells at a finer resolution that fit within the given cell.
 * @param cell (optional) — H3 cell index
 * @param childResolution (optional) — Target resolution for children (must be higher than cell's resolution)
 * @returns children — Array of child H3 cell indices
 * @returns count — Number of child cells
 */
declare function h3CellToChildren({ cell?: string, childResolution?: int }): { children: string[], count: int };

/**
 * Converts an H3 cell index to the geographic coordinate of its center point.
 * @param cell (optional) — H3 cell index as a hexadecimal string
 * @returns coordinate — The center coordinate of the H3 cell
 */
declare function h3CellToLatlng({ cell?: string }): Struct;

/**
 * Returns the parent cell at a coarser resolution. The parent contains the given cell.
 * @param cell (optional) — H3 cell index
 * @param parentResolution (optional) — Target resolution for the parent (must be lower than cell's resolution)
 * @returns parent — Parent H3 cell index at the specified resolution
 * @returns originalResolution — Resolution of the input cell
 */
declare function h3CellToParent({ cell?: string, parentResolution?: int }): { parent: string, originalResolution: int };

/**
 * Converts a set of H3 cells to polygon boundaries. Returns the outline(s) of the cell set, merging adjacent cells.
 * @param cells (optional) — Array of H3 cell indices
 * @returns polygons — Array of polygons representing the merged cell boundaries
 * @returns polygonCount — Number of separate polygons (disconnected regions)
 */
declare function h3CellsToMultiPolygon({ cells?: string[] }): { polygons: Struct, polygonCount: int };

/**
 * Compacts a set of H3 cells by replacing groups of cells with their parent when all children are present. Reduces the number of cells while covering the same area.
 * @param cells (optional) — Array of H3 cell indices to compact
 * @returns compacted — Array of compacted H3 cell indices (may contain mixed resolutions)
 * @returns originalCount — Number of input cells
 * @returns compactedCount — Number of cells after compaction
 */
declare function h3CompactCells({ cells?: string[] }): { compacted: string[], originalCount: int, compactedCount: int };

/**
 * Returns the average edge length of H3 cells at a given resolution.
 * @param resolution (optional) — H3 resolution (0-15)
 * @param unit (optional) — Length unit for the result
 * @returns edgeLength — Average edge length at this resolution
 * @returns cellCount — Total number of cells at this resolution covering Earth
 */
declare function h3EdgeLength({ resolution?: int, unit?: Struct }): { edgeLength: float, cellCount: int };

/**
 * Returns all H3 cells within k steps of the origin cell (a filled disk of hexagons). Useful for proximity searches and area coverage.
 * @param cell (optional) — Origin H3 cell index
 * @param k (optional) — Number of rings around the origin (0 = just the origin cell)
 * @returns cells — Array of H3 cell indices in the disk
 * @returns count — Number of cells in the disk
 */
declare function h3GridDisk({ cell?: string, k?: int }): { cells: string[], count: int };

/**
 * Calculates the grid distance (number of steps) between two H3 cells. Both cells must be at the same resolution.
 * @param cellA (optional) — First H3 cell index
 * @param cellB (optional) — Second H3 cell index
 * @returns distance — Grid distance (number of hexagon steps) between the cells
 */
declare function h3GridDistance({ cellA?: string, cellB?: string }): int;

/**
 * Finds a path of H3 cells between two cells. Returns all cells along the shortest path. Both cells must be at the same resolution.
 * @param cellA (optional) — Starting H3 cell index
 * @param cellB (optional) — Ending H3 cell index
 * @returns path — Array of H3 cell indices along the path (including start and end)
 * @returns length — Number of cells in the path
 */
declare function h3GridPath({ cellA?: string, cellB?: string }): { path: string[], length: int };

/**
 * Converts a geographic coordinate to an H3 cell index at the specified resolution. H3 is a hierarchical hexagonal grid system.
 * @param coordinate — The geographic coordinate (latitude, longitude)
 * @param resolution (optional) — H3 resolution (0-15). Higher = smaller cells. 0 = ~4,357,449 km², 15 = ~0.9 m²
 * @returns cell — H3 cell index as a hexadecimal string
 */
declare function h3LatlngToCell({ coordinate: Struct, resolution?: int }): string;


// === Web/Geo/Map ===

/**
 * Fetches a static map image for the given coordinates using OpenStreetMap tiles. Returns a satellite/standard map image centered on the location.
 * @param coordinate — The geographic coordinate (latitude, longitude) to center the map on
 * @param zoom (optional) — Map zoom level (1-19). Higher values show more detail. Default: 15
 * @param width (optional) — Image width in pixels. Default: 512
 * @param height (optional) — Image height in pixels. Default: 512
 * @param style (optional) — Map style to use
 * @returns image — The fetched map image
 * @impure has side effects / drives control flow
 */
declare function geoGetMapImage({ coordinate: Struct, zoom?: int, width?: int, height?: int, style?: string }): Struct;


// === Web/Geo/Routing ===

/**
 * Snaps noisy GPS traces to the road network using OSRM map matching.
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
declare function geoOsrmMatchTrace({ coordinates?: Struct[], profile?: Struct, timestamps?: int[], radiuses?: float[], gaps?: string, tidy?: bool, baseUrl?: string }): { matchings: Struct[], primaryMatching: Struct, tracepoints: Struct[] };

/**
 * Finds the nearest routable point(s) to a coordinate using OSRM.
 * @param coordinate — The coordinate to snap to the road network
 * @param profile (optional) — Transportation mode: Car, Bike, or Foot
 * @param number (optional) — Maximum number of nearest points to return (1-50)
 * @param baseUrl (optional) — OSRM server base URL
 * @returns nearest — The closest routable point
 * @returns waypoints — List of nearest routable points
 * @impure has side effects / drives control flow
 */
declare function geoOsrmNearest({ coordinate: Struct, profile?: Struct, number?: int, baseUrl?: string }): { nearest: Struct, waypoints: Struct[] };

/**
 * Computes travel time and distance matrices between coordinates using OSRM.
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
declare function geoOsrmTable({ coordinates?: Struct[], profile?: string, sources?: int[], destinations?: int[], includeDurations?: bool, includeDistances?: bool, baseUrl?: string }): { durations: Struct[], distances: Struct[], result: Struct };

/**
 * Fetches vector map tiles (MVT) from an OSRM server.
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
declare function geoOsrmTile({ profile?: Struct, z?: int, x?: int, y?: int, path: Struct, baseUrl?: string }): { tilePath: Struct, contentType: string };

/**
 * Plans the shortest round trip through multiple coordinates using OSRM.
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
declare function geoOsrmTrip({ coordinates?: Struct[], profile?: Struct, roundtrip?: bool, source?: string, destination?: string, baseUrl?: string }): { trip: Struct, trips: Struct[], waypoints: Struct[], distance: float, duration: float, geometry: Struct[] };

/**
 * Plans a route between two points using the OSRM routing service. Returns turn-by-turn directions, distance, and duration.
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
declare function geoPlanRoute({ start: Struct, end: Struct, waypoints?: Struct, profile?: string, alternatives?: bool }): { route: Struct, alternativesOut: Struct, distance: float, duration: float, geometry: Struct };


// === Web/Geo/Search ===

/**
 * Converts geographic coordinates to a human-readable address using the Nominatim service (OpenStreetMap).
 * @param coordinate — The geographic coordinate (latitude, longitude) to look up
 * @param zoom (optional) — Level of detail for the address (0-18). Higher = more specific. Default: 18
 * @returns result — The reverse geocoding result with address details
 * @returns displayName — The full formatted address string
 * @impure has side effects / drives control flow
 */
declare function geoReverseGeocode({ coordinate: Struct, zoom?: int }): { result: Struct, displayName: string };

/**
 * Searches for a location by name or address using the Nominatim geocoding service (OpenStreetMap). Returns matching locations with coordinates.
 * @param query (optional) — The search query (address, place name, etc.)
 * @param limit (optional) — Maximum number of results to return. Default: 5
 * @param countryCodes (optional) — Optional comma-separated list of country codes to limit search (e.g., 'de,at,ch')
 * @returns results — Array of search results with coordinates
 * @returns firstResult — The first/best matching result (if any)
 * @impure has side effects / drives control flow
 */
declare function geoSearchLocation({ query?: string, limit?: int, countryCodes?: string }): { results: Struct[], firstResult: Struct };


// === Web/MCP ===

/**
 * Registers MCP server authentication settings.
 * @param configIn — MCP server config
 * @param auth — Auth config
 * @returns configOut — Updated config
 */
declare function mcpRegisterAuth({ configIn: Struct, auth: Struct }): Struct;

/**
 * Registers referenced Flow functions as MCP tools.
 * @param configIn — MCP server config
 * @returns configOut — Updated config
 */
declare function mcpRegisterFunctions({ configIn: Struct }): Struct;

/**
 * Registers a static MCP prompt template.
 * @param configIn — MCP server config
 * @param name — Prompt name
 * @param description — Optional description
 * @param template — Prompt template
 * @returns configOut — Updated config
 */
declare function mcpRegisterPrompt({ configIn: Struct, name: string, description: string, template: string }): Struct;

/**
 * Registers a FlowPath as an MCP resource.
 * @param configIn — MCP server config
 * @param flowPath — Resource FlowPath
 * @param uri — MCP resource URI exposed to clients. Defaults to file://<flow path> when empty.
 * @param name — Resource display name. Defaults to the FlowPath filename when empty.
 * @param description — Optional description
 * @returns configOut — Updated config
 */
declare function mcpRegisterResource({ configIn: Struct, flowPath: Struct, uri: string, name: string, description: string }): Struct;

/**
 * Starts an MCP server from a composed config.
 * @param config — MCP server config
 * @returns localAddr — Bound address
 * @impure has side effects / drives control flow
 */
declare function mcpServer({ config: Struct }): string;

/**
 * Creates an MCP server config that function, resource, prompt, auth, and server nodes can compose.
 * @param host (optional) — Bind host
 * @param port (optional) — Bind port
 * @param path (optional) — MCP HTTP path
 * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
 * @param maxConnections (optional) — Maximum concurrent requests
 * @param maxBodyBytes (optional) — Maximum HTTP request body size
 * @param tls — TLS security config
 * @returns config — MCP server config
 */
declare function mcpServerConfig({ host?: string, port?: int, path?: string, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;


// === Web/MQTT ===

/**
 * Binds a lightweight MQTT broker for daemon workflows. Typed lifecycle events are exposed as pins; published messages are delivered to the referenced on-message handler.
 * @param config — MQTT broker configuration
 * @returns localAddr — Bound broker socket address
 * @returns clientId — Connected MQTT client id
 * @returns remoteAddr — Remote client socket address
 * @impure has side effects / drives control flow
 */
declare function mqttBroker({ config: Struct }): { localAddr: string, clientId: string, remoteAddr: string };

/**
 * Connects to an MQTT broker and returns a session reference for use with Publish, Subscribe, and Disconnect nodes.
 * @param config — MQTT connection configuration (host, port, client_id, optional credentials, TLS)
 * @returns session — MQTT session reference for use with Publish/Subscribe/Disconnect nodes
 * @impure has side effects / drives control flow
 */
declare function mqttConnect({ config: Struct }): Struct;

/**
 * Disconnects from an MQTT broker and cleans up the session
 * @param session — MQTT session to disconnect
 * @impure has side effects / drives control flow
 */
declare function mqttDisconnect({ session: Struct }): void;

/**
 * Publishes a message to an MQTT topic
 * @param session — MQTT session reference
 * @param topic — The MQTT topic to publish to
 * @param payload — The message content to publish
 * @param qos (optional) — Quality of Service level
 * @param retain (optional) — Whether the broker should retain this message
 * @impure has side effects / drives control flow
 */
declare function mqttPublish({ session: Struct, topic: string, payload: string, qos?: string, retain?: bool }): void;

/**
 * Subscribes to an MQTT topic and invokes a handler for each incoming message. Holds execution until the connection closes or timeout, then triggers on_close.
 * @param session — MQTT session reference
 * @param topic — The MQTT topic filter to subscribe to
 * @param qos (optional) — Quality of Service level for the subscription
 * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
 * @impure has side effects / drives control flow
 */
declare function mqttSubscribe({ session: Struct, topic: string, qos?: string, timeoutSeconds?: int }): void;


// === Web/REST ===

/**
 * Registers REST server authentication settings.
 * @param configIn — REST server config
 * @param auth (optional) — Auth config
 * @returns configOut — Updated config
 */
declare function restRegisterAuth({ configIn: Struct, auth?: Struct }): Struct;

/**
 * Registers a FlowPath file or directory as static REST responses.
 * @param configIn — REST server config
 * @param path — HTTP route path
 * @param flowPath — File or directory FlowPath
 * @param directory (optional) — Serve the FlowPath as a directory mount
 * @param contentType (optional) — Optional response content type override
 * @returns configOut — Updated config
 */
declare function restRegisterFiles({ configIn: Struct, path: string, flowPath: Struct, directory?: bool, contentType?: string }): Struct;

/**
 * Registers referenced Flow functions as handlers for a REST path.
 * @param configIn — REST server config
 * @param path — HTTP route path
 * @param method (optional) — Allowed HTTP method. ANY accepts all methods.
 * @returns configOut — Updated config
 */
declare function restRegisterFunction({ configIn: Struct, path: string, method?: string }): Struct;

/**
 * Registers OpenAPI JSON and browser UI endpoints generated from the REST server config.
 * @param configIn — REST server config
 * @param path (optional) — OpenAPI JSON route path
 * @param uiPath (optional) — OpenAPI browser UI route path; empty disables the UI
 * @returns configOut — Updated config
 */
declare function restRegisterOpenApi({ configIn: Struct, path?: string, uiPath?: string }): Struct;

/**
 * Starts a REST server from a composed config. Function routes and files are registered on the config before this node runs.
 * @param config — REST server config
 * @returns localAddr — Bound address
 * @impure has side effects / drives control flow
 */
declare function restServer({ config: Struct }): string;

/**
 * Creates a REST server config that route, file, auth, and server nodes can compose.
 * @param host (optional) — Bind host
 * @param port (optional) — Bind port
 * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
 * @param maxConnections (optional) — Maximum concurrent requests
 * @param maxBodyBytes (optional) — Maximum HTTP request body size
 * @param tls — TLS security config
 * @returns config — REST server config
 */
declare function restServerConfig({ host?: string, port?: int, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;


// === Web/Scraping ===

/**
 * Extracts links from the input text
 * @param startingPage — The page to start extracting links from
 * @param sameDomain (optional) — Stay on the same domain or subdomains
 * @param offsetMs (optional) — Delay between requests
 * @param depth (optional) — The depth to extract links from
 * @returns links — The extracted links
 * @impure has side effects / drives control flow
 */
declare function webScrapeExtractLinks({ startingPage: string, sameDomain?: bool, offsetMs?: int, depth?: int }): Set<string>;


// === Web/TCP ===

/**
 * Closes an open TCP connection gracefully
 * @param session — TCP session to close
 * @impure has side effects / drives control flow
 */
declare function tcpClose({ session: Struct }): void;

/**
 * Opens a TCP connection to a remote host. Triggers on_connect with the session, then invokes the on-message handler for each incoming data chunk. Holds execution until the connection closes, then triggers on_close.
 * @param config — TCP connection configuration (host, port, optional timeout)
 * @returns session — TCP session reference for use with Send/Close nodes
 * @impure has side effects / drives control flow
 */
declare function tcpConnect({ config: Struct }): Struct;

/**
 * Binds a TCP listener on a port. Fires on_listening, then accepts incoming connections and invokes the handler for each. Holds execution until closed or timed out, then triggers on_close.
 * @param config — TCP listener configuration (host, port, optional timeout, max connections)
 * @impure has side effects / drives control flow
 */
declare function tcpListen({ config: Struct }): void;

/**
 * Sends data through an open TCP connection
 * @param session — TCP session reference
 * @param messageType (optional) — Whether to send as text (UTF-8) or binary
 * @param payload — The data to send (string for Text, byte array for Binary)
 * @impure has side effects / drives control flow
 */
declare function tcpSend({ session: Struct, messageType?: string, payload: string }): void;

/**
 * Binds a TCP server. Typed lifecycle events are exposed as pins; incoming data chunks are delivered to the referenced on-message handler.
 * @param config — TCP server configuration
 * @returns localAddr — Bound local socket address
 * @returns session — Accepted TCP client session
 * @returns remoteAddr — Remote client socket address
 * @impure has side effects / drives control flow
 */
declare function tcpServer({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };


// === Web/TLS ===

/**
 * Creates a local certificate authority certificate and private key.
 * @param commonName (optional) — Certificate authority common name
 * @returns certificate — Certificate authority PEM bundle
 * @impure has side effects / drives control flow
 */
declare function createCaCertificate({ commonName?: string }): Struct;

/**
 * Creates a server or client certificate signed by a local certificate authority.
 * @param ca — Certificate authority PEM bundle
 * @param commonName (optional) — Certificate common name
 * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
 * @param usage (optional) — Certificate usage
 * @returns certificate — Signed certificate PEM bundle
 * @impure has side effects / drives control flow
 */
declare function createCaSignedCertificate({ ca: Struct, commonName?: string, subjectAltNames?: string[], usage?: string }): Struct;

/**
 * Creates a self-signed certificate and private key.
 * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
 * @returns certificate — Self-signed certificate PEM bundle
 * @impure has side effects / drives control flow
 */
declare function createSelfSignedCertificate({ subjectAltNames?: string[] }): Struct;


// === Web/UDP ===

/**
 * Binds a UDP socket to a local address and port
 * @param config — UDP bind configuration (host and port)
 * @returns session — UDP session reference for use with SendTo/Receive/Close nodes
 * @impure has side effects / drives control flow
 */
declare function udpBind({ config: Struct }): Struct;

/**
 * Closes a bound UDP socket and releases resources
 * @param session — UDP session to close
 * @impure has side effects / drives control flow
 */
declare function udpClose({ session: Struct }): void;

/**
 * Listens for incoming datagrams on a bound UDP socket. Invokes the on-message handler for each received datagram. Holds execution until the socket is closed or the timeout expires, then fires on_close.
 * @param session — UDP session reference from a Bind node
 * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
 * @impure has side effects / drives control flow
 */
declare function udpReceive({ session: Struct, timeoutSeconds?: int }): void;

/**
 * Sends a datagram to a target address through a bound UDP socket
 * @param session — UDP session reference
 * @param targetHost — Destination host address
 * @param targetPort — Destination port number
 * @param payload — The message content to send
 * @returns bytesSent — Number of bytes sent
 * @impure has side effects / drives control flow
 */
declare function udpSendTo({ session: Struct, targetHost: string, targetPort: int, payload: string }): int;

/**
 * Binds a UDP socket. Typed lifecycle pins describe the socket; incoming datagrams are delivered to the referenced on-message handler.
 * @param config — UDP server configuration
 * @returns session — UDP server socket session
 * @returns localAddr — Bound local socket address
 * @impure has side effects / drives control flow
 */
declare function udpServer({ config: Struct }): { session: Struct, localAddr: string };


// === Web/WebSocket ===

/**
 * Closes an open WebSocket connection gracefully
 * @param session — WebSocket session to close
 * @impure has side effects / drives control flow
 */
declare function websocketClose({ session: Struct }): void;

/**
 * Opens a WebSocket connection. Immediately triggers on_connect with the session, then invokes on_message for each incoming message. Holds execution until the connection closes, then triggers on_close.
 * @param config — WebSocket connection configuration (URL, optional headers, optional timeout)
 * @returns session — WebSocket session reference for use with Send/Close nodes
 * @impure has side effects / drives control flow
 */
declare function websocketConnect({ config: Struct }): Struct;

/**
 * Sends a message through an open WebSocket connection
 * @param session — WebSocket session reference
 * @param messageType (optional) — Whether to send as text or binary
 * @param payload — The message content to send (string for Text, byte array for Binary)
 * @impure has side effects / drives control flow
 */
declare function websocketSend({ session: Struct, messageType?: string, payload: string }): void;

/**
 * Binds a WebSocket server. Typed lifecycle events are exposed as pins; incoming messages are delivered to the referenced on-message handler.
 * @param config — WebSocket server configuration
 * @returns localAddr — Bound local socket address
 * @returns session — Accepted WebSocket client session
 * @returns remoteAddr — Remote client socket address
 * @impure has side effects / drives control flow
 */
declare function websocketServer({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };

