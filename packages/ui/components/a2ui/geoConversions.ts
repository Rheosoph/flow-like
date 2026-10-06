import type * as GeoJSON from "geojson";
import {
	GEOMETRY_KINDS,
	type Geometry,
	geometryMarker,
	normalizeGeometryValue,
} from "../../lib/geometry";
import type {
	GeoCoordinate,
	GeoMapMarkerDef,
	GeoMapRouteDef,
	GeoMapViewport,
	GeoRouteResult,
	GeoSearchResult,
	GeoTripWaypoint,
} from "./types";

let idCounter = 0;
function nextId(prefix: string): string {
	return `${prefix}-${++idCounter}`;
}

export function routeResultToRouteDef(
	route: GeoRouteResult,
	options?: { id?: string; color?: string; width?: number; opacity?: number },
): GeoMapRouteDef {
	return {
		id: options?.id ?? nextId("route"),
		coordinates: route.geometry.points,
		color: options?.color,
		width: options?.width,
		opacity: options?.opacity,
	};
}

export function routeResultsToRouteDefs(
	routes: GeoRouteResult[],
	options?: { color?: string; width?: number; opacity?: number },
): GeoMapRouteDef[] {
	return routes.map((r, i) =>
		routeResultToRouteDef(r, { ...options, id: `route-${i}` }),
	);
}

export function searchResultToMarkerDef(
	result: GeoSearchResult,
	options?: { id?: string; color?: string; draggable?: boolean },
): GeoMapMarkerDef {
	return {
		id: options?.id ?? nextId("search"),
		coordinate: result.coordinate,
		label: result.display_name,
		popup: `${result.display_name} (${result.place_type})`,
		color: options?.color ?? "blue",
		draggable: options?.draggable ?? false,
	};
}

export function searchResultsToMarkerDefs(
	results: GeoSearchResult[],
	options?: { color?: string },
): GeoMapMarkerDef[] {
	return results.map((r, i) =>
		searchResultToMarkerDef(r, { ...options, id: `search-${i}` }),
	);
}

export function tripWaypointToMarkerDef(
	waypoint: GeoTripWaypoint,
	options?: { id?: string; color?: string; draggable?: boolean },
): GeoMapMarkerDef {
	return {
		id: options?.id ?? nextId("waypoint"),
		coordinate: waypoint.coordinate,
		label: waypoint.name || undefined,
		color: options?.color ?? "green",
		draggable: options?.draggable ?? false,
	};
}

export function tripWaypointsToMarkerDefs(
	waypoints: GeoTripWaypoint[],
	options?: { color?: string },
): GeoMapMarkerDef[] {
	return waypoints.map((w, i) =>
		tripWaypointToMarkerDef(w, { ...options, id: `waypoint-${i}` }),
	);
}

export function routeStepsToMarkerDefs(
	route: GeoRouteResult,
	options?: { color?: string },
): GeoMapMarkerDef[] {
	return route.legs.flatMap((leg, li) =>
		leg.steps.map((step, si) => ({
			id: `step-${li}-${si}`,
			coordinate: step.coordinate,
			label: step.name || undefined,
			popup: step.instruction,
			color: options?.color ?? "orange",
		})),
	);
}

export function searchResultToViewport(
	result: GeoSearchResult,
): GeoMapViewport {
	return {
		center: result.coordinate,
		zoom: result.bounding_box ? 14 : 12,
	};
}

/** Map events keep latitude/longitude objects even when the displayed value is Geometry. */
export function mapEventCoordinate(
	longitude: number,
	latitude: number,
): GeoCoordinate {
	return { latitude, longitude };
}

function record(value: unknown): Record<string, unknown> | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

function nativeGeometry(value: unknown): Geometry | undefined {
	const object = record(value);
	if (!object || typeof object.type !== "string") return undefined;
	// Search results use `type` for place categories, alongside a legacy coordinate.
	if (
		object.coordinate !== undefined &&
		object.coordinates === undefined &&
		object.geometries === undefined &&
		!GEOMETRY_KINDS.some((kind) => kind === object.type) &&
		object.type !== "Feature" &&
		object.type !== "FeatureCollection"
	)
		return undefined;
	// The shared validator rejects Feature wrappers, invalid nesting and unsupported dimensions.
	return normalizeGeometryValue(value) as Geometry;
}

function coordinate(value: unknown): GeoCoordinate {
	const object = record(value);
	if (object?.type !== undefined) {
		const geometry = normalizeGeometryValue(value, {
			schema: geometryMarker("Point"),
		}) as GeoJSON.Point;
		return mapEventCoordinate(geometry.coordinates[0], geometry.coordinates[1]);
	}
	const longitude = object?.longitude;
	const latitude = object?.latitude;
	if (
		typeof longitude !== "number" ||
		typeof latitude !== "number" ||
		!Number.isFinite(longitude) ||
		!Number.isFinite(latitude)
	)
		throw new Error("Map coordinates must be finite numbers");
	// Legacy map values retain wrapped longitudes emitted while panning across world copies.
	return mapEventCoordinate(longitude, latitude);
}

function geometryMarkers(geometry: Geometry, id: string): GeoMapMarkerDef[] {
	switch (geometry.type) {
		case "Point":
			return [
				{
					id,
					coordinate: mapEventCoordinate(
						geometry.coordinates[0],
						geometry.coordinates[1],
					),
				},
			];
		case "MultiPoint":
			return geometry.coordinates.map((position, index) => ({
				id: `${id}-${index}`,
				coordinate: mapEventCoordinate(position[0], position[1]),
			}));
		case "GeometryCollection":
			return geometry.geometries.flatMap((item, index) =>
				geometryMarkers(item, `${id}-${index}`),
			);
		default:
			return [];
	}
}

function geometryRoutes(geometry: Geometry, id: string): GeoMapRouteDef[] {
	const line = (positions: number[][], key: string): GeoMapRouteDef => ({
		id: key,
		coordinates: positions.map(([longitude, latitude]) =>
			mapEventCoordinate(longitude, latitude),
		),
	});
	switch (geometry.type) {
		case "LineString":
			return [line(geometry.coordinates, id)];
		case "MultiLineString":
		case "Polygon":
			return geometry.coordinates.map((positions, index) =>
				line(positions, `${id}-${index}`),
			);
		case "MultiPolygon":
			return geometry.coordinates.flatMap((polygon, pi) =>
				polygon.map((positions, ri) => line(positions, `${id}-${pi}-${ri}`)),
			);
		case "GeometryCollection":
			return geometry.geometries.flatMap((item, index) =>
				geometryRoutes(item, `${id}-${index}`),
			);
		default:
			return [];
	}
}

/** Native polygons render their exterior and hole outlines; points render markers. */
export function normalizeGeoMapMarkers(raw: unknown): GeoMapMarkerDef[] {
	if (raw == null) return [];
	const values = Array.isArray(raw) ? raw : [raw];
	return values.flatMap((value, index) => {
		const id = `geometry-marker-${index}`;
		const geometry = nativeGeometry(value);
		if (geometry) return geometryMarkers(geometry, id);
		const object = record(value);
		if (!object)
			throw new Error("A map marker requires a Point or a coordinate object");
		const position = object.coordinate ?? object.geometry;
		return [
			{
				...object,
				id: typeof object.id === "string" ? object.id : id,
				coordinate: coordinate(position),
				label:
					typeof object.label === "string"
						? object.label
						: typeof object.display_name === "string"
							? object.display_name
							: undefined,
			} as GeoMapMarkerDef,
		];
	});
}

export function normalizeGeoMapRoutes(raw: unknown): GeoMapRouteDef[] {
	if (raw == null) return [];
	const values = Array.isArray(raw) ? raw : [raw];
	return values.flatMap((value, index) => {
		const id = `geometry-route-${index}`;
		const geometry = nativeGeometry(value);
		if (geometry) return geometryRoutes(geometry, id);
		const object = record(value);
		if (!object)
			throw new Error("A map route requires a geometry or a route object");
		const nested = nativeGeometry(object.geometry);
		if (nested)
			return geometryRoutes(
				nested,
				typeof object.id === "string" ? object.id : id,
			).map((route) => ({ ...object, ...route }) as GeoMapRouteDef);
		const positions =
			object.coordinates ?? record(object.geometry)?.points ?? object.points;
		if (!Array.isArray(positions))
			throw new Error("A map route requires coordinate positions");
		return [
			{
				...object,
				id: typeof object.id === "string" ? object.id : id,
				coordinates: positions.map(coordinate),
			} as GeoMapRouteDef,
		];
	});
}

/** Resolve current nested, legacy flat, wrapped literal and native Point viewports. */
export function geoMapViewportValue(raw: unknown): GeoMapViewport | undefined {
	if (raw == null) return undefined;
	const object = record(raw);
	if (typeof object?.literalJson === "string")
		return geoMapViewportValue(JSON.parse(object.literalJson));
	if (!object) throw new Error("A map viewport requires a center coordinate");
	const center =
		object.type !== undefined
			? coordinate(object)
			: coordinate(object.center ?? object);
	const viewport: GeoMapViewport = { center };
	for (const key of ["zoom", "bearing", "pitch"] as const) {
		const value = object[key];
		if (value == null) continue;
		if (typeof value !== "number" || !Number.isFinite(value))
			throw new Error(`Map viewport ${key} must be finite`);
		viewport[key] = value;
	}
	return viewport;
}
