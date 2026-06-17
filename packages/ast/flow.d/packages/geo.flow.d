// geo — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

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

