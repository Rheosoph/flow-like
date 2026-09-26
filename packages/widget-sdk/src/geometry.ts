import type { JsonSchema } from "./contract";

/** A WGS 84 position: longitude [-180, 180], then latitude [-90, 90]. */
export type GeoPosition = [longitude: number, latitude: number];

interface GeoMembers {
	bbox?: [west: number, south: number, east: number, north: number];
	[member: string]: unknown;
}

/** @geometry Point */
export interface GeoPoint extends GeoMembers {
	type: "Point";
	coordinates: GeoPosition;
}

/** @geometry LineString */
export interface GeoLineString extends GeoMembers {
	type: "LineString";
	coordinates: GeoPosition[];
}

/** @geometry Polygon */
export interface GeoPolygon extends GeoMembers {
	type: "Polygon";
	coordinates: GeoPosition[][];
}

/** @geometry MultiPoint */
export interface GeoMultiPoint extends GeoMembers {
	type: "MultiPoint";
	coordinates: GeoPosition[];
}

/** @geometry MultiLineString */
export interface GeoMultiLineString extends GeoMembers {
	type: "MultiLineString";
	coordinates: GeoPosition[][];
}

/** @geometry MultiPolygon */
export interface GeoMultiPolygon extends GeoMembers {
	type: "MultiPolygon";
	coordinates: GeoPosition[][][];
}

/** @geometry GeometryCollection */
export interface GeoGeometryCollection extends GeoMembers {
	type: "GeometryCollection";
	geometries: GeoGeometry[];
}

/** @geometry Any */
export type GeoGeometry =
	| GeoPoint
	| GeoLineString
	| GeoPolygon
	| GeoMultiPoint
	| GeoMultiLineString
	| GeoMultiPolygon
	| GeoGeometryCollection;

type GeometryKind = GeoGeometry["type"];
const GEOMETRY_KINDS: readonly GeometryKind[] = [
	"Point",
	"LineString",
	"Polygon",
	"MultiPoint",
	"MultiLineString",
	"MultiPolygon",
	"GeometryCollection",
];

// Keep these limits aligned with flow_like_types_contracts::geometry.
const MAX_GEOMETRY_BYTES = 1_048_576;
const MAX_GEOMETRY_DEPTH = 32;
const MAX_GEOMETRY_POSITIONS = 100_000;
const MAX_GEOMETRY_MEMBERS = 1_000_000;
const encoder = new TextEncoder();

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isGeometryKind(value: unknown): value is GeometryKind {
	return GEOMETRY_KINDS.includes(value as GeometryKind);
}

function checkJsonProfile(value: unknown): void {
	let members = 0;
	let bytes = 0;
	const visit = (item: unknown, depth: number): void => {
		if (depth > MAX_GEOMETRY_DEPTH || ++members > MAX_GEOMETRY_MEMBERS) {
			throw new Error("Geometry exceeds the depth or member limit");
		}
		if (Array.isArray(item)) {
			for (const child of item) visit(child, depth + 1);
		} else if (isObject(item)) {
			const prototype = Object.getPrototypeOf(item);
			if (prototype !== Object.prototype && prototype !== null) {
				throw new Error("Geometry must contain JSON objects");
			}
			for (const [key, child] of Object.entries(item)) {
				bytes += encoder.encode(key).length;
				visit(child, depth + 1);
			}
		} else if (typeof item === "string") {
			bytes += encoder.encode(item).length;
		} else if (typeof item === "number") {
			if (!Number.isFinite(item)) {
				throw new Error("Geometry numbers must be finite");
			}
			bytes++;
		} else if (item === null || typeof item === "boolean") {
			bytes++;
		} else {
			throw new Error("Geometry must contain JSON values");
		}
		if (bytes > MAX_GEOMETRY_BYTES) {
			throw new Error("Geometry exceeds the byte limit");
		}
	};
	visit(value, 0);
	if (encoder.encode(JSON.stringify(value)).length > MAX_GEOMETRY_BYTES) {
		throw new Error("Geometry exceeds the byte limit");
	}
}

function checkGeometry(value: unknown, expected?: GeometryKind): void {
	checkJsonProfile(value);
	let positions = 0;
	const array = (item: unknown, path: string): unknown[] => {
		if (!Array.isArray(item)) throw new Error(`${path} must be an array`);
		return item;
	};
	const position = (item: unknown, path: string, count = true): GeoPosition => {
		const coordinates = array(item, path);
		const [longitude, latitude] = coordinates;
		if (
			coordinates.length !== 2 ||
			typeof longitude !== "number" ||
			typeof latitude !== "number" ||
			!Number.isFinite(longitude) ||
			!Number.isFinite(latitude)
		) {
			throw new Error(
				`${path} requires two finite numbers: longitude, latitude`,
			);
		}
		if (
			longitude < -180 ||
			longitude > 180 ||
			latitude < -90 ||
			latitude > 90
		) {
			throw new Error(`${path} is outside WGS 84 longitude/latitude bounds`);
		}
		if (count && ++positions > MAX_GEOMETRY_POSITIONS) {
			throw new Error("Geometry exceeds the position limit");
		}
		return [longitude, latitude];
	};
	const line = (item: unknown, path: string, ring = false): void => {
		const coordinates = array(item, path);
		const minimum = ring ? 4 : 2;
		if (coordinates.length < minimum) {
			throw new Error(`${path} needs at least ${minimum} positions`);
		}
		let first: GeoPosition | undefined;
		let last: GeoPosition | undefined;
		coordinates.forEach((item, index) => {
			last = position(item, `${path}[${index}]`);
			first ??= last;
		});
		if (
			ring &&
			first &&
			last &&
			(first[0] !== last[0] || first[1] !== last[1])
		) {
			throw new Error(`${path} must be closed`);
		}
	};
	const polygon = (item: unknown, path: string): void => {
		const rings = array(item, path);
		if (rings.length === 0) throw new Error(`${path} needs an exterior ring`);
		rings.forEach((ring, index) => line(ring, `${path}[${index}]`, true));
	};
	const geometry = (item: unknown, path: string, kind?: GeometryKind): void => {
		if (!isObject(item) || !isGeometryKind(item.type)) {
			throw new Error(`${path} must be a GeoJSON geometry object`);
		}
		if (kind && item.type !== kind) {
			throw new Error(`${path} must be ${kind}, received ${item.type}`);
		}
		if ("crs" in item) {
			throw new Error(`${path}: alternate CRS declarations are unsupported`);
		}
		if ("bbox" in item) {
			const bbox = array(item.bbox, `${path}.bbox`);
			if (bbox.length !== 4) {
				throw new Error(`${path}.bbox requires four coordinates`);
			}
			const southwest = position(
				bbox.slice(0, 2),
				`${path}.bbox southwest`,
				false,
			);
			const northeast = position(
				bbox.slice(2),
				`${path}.bbox northeast`,
				false,
			);
			if (southwest[1] > northeast[1]) {
				throw new Error(`${path}.bbox south must not exceed north`);
			}
		}
		if (item.type === "GeometryCollection") {
			if ("coordinates" in item) {
				throw new Error(`${path}: GeometryCollection uses geometries`);
			}
			array(item.geometries, `${path}.geometries`).forEach((child, index) =>
				geometry(child, `${path}.geometries[${index}]`),
			);
			return;
		}
		if ("geometries" in item) {
			throw new Error(
				`${path}: only GeometryCollection may contain geometries`,
			);
		}
		const pathCoordinates = `${path}.coordinates`;
		switch (item.type) {
			case "Point":
				position(item.coordinates, pathCoordinates);
				break;
			case "LineString":
				line(item.coordinates, pathCoordinates);
				break;
			case "Polygon":
				polygon(item.coordinates, pathCoordinates);
				break;
			case "MultiPoint":
				array(item.coordinates, pathCoordinates).forEach((point, index) =>
					position(point, `${pathCoordinates}[${index}]`),
				);
				break;
			case "MultiLineString":
				array(item.coordinates, pathCoordinates).forEach((item, index) =>
					line(item, `${pathCoordinates}[${index}]`),
				);
				break;
			case "MultiPolygon":
				array(item.coordinates, pathCoordinates).forEach((item, index) =>
					polygon(item, `${pathCoordinates}[${index}]`),
				);
		}
	};
	geometry(value, "Geometry", expected);
}

/** Apply the geometry profile when a contract schema carries a geometry marker. */
export function checkGeometrySchema(schema: JsonSchema, value: unknown): void {
	const compact = schema.$id === "flow:geometry";
	const full = schema["x-flow-like-type"] === "geometry";
	if (!compact && !full && !("x-geometry" in schema)) return;
	const subtype = schema["x-geometry"];
	if (
		(compact &&
			(Object.keys(schema).length !== 2 || !isGeometryKind(subtype))) ||
		(!compact && !full) ||
		("x-geometry" in schema && !isGeometryKind(subtype))
	) {
		throw new Error("Invalid geometry subtype marker");
	}
	checkGeometry(value, isGeometryKind(subtype) ? subtype : undefined);
}
