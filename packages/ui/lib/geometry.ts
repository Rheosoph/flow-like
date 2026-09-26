import { IValueType } from "./schema/flow/pin";

/** Frozen wire markers shared with flow_like_types_contracts::geometry. */
export const GEOMETRY_SCHEMA_ID = "flow:geometry";
export const GEOMETRY_KINDS = [
	"Point",
	"LineString",
	"Polygon",
	"MultiPoint",
	"MultiLineString",
	"MultiPolygon",
	"GeometryCollection",
] as const;
export type GeometryKind = (typeof GEOMETRY_KINDS)[number];
export type Geometry = GeoJSON.Geometry;

export const MAX_GEOMETRY_JSON_BYTES = 1_048_576;
export const MAX_GEOMETRY_DEPTH = 32;
export const MAX_GEOMETRY_POSITIONS = 100_000;
export const MAX_GEOMETRY_MEMBERS = 1_000_000;

const isObject = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

/** Mirrors storage's `names_geometry_kind`: the shape a decoded geoarrow value arrives in. */
export function namesGeometryKind(value: unknown): boolean {
	return isObject(value) && GEOMETRY_KINDS.includes(value.type as GeometryKind);
}

export function geometryMarker(kind?: GeometryKind | null): string | null {
	return kind
		? JSON.stringify({ $id: GEOMETRY_SCHEMA_ID, "x-geometry": kind })
		: null;
}

/** Missing schema means any geometry; malformed markers are errors. */
export function geometryKindFromSchema(
	schema?: string | null,
	refs?: Record<string, string>,
): GeometryKind | null {
	if (schema == null) return null;
	const visited = new Set<string>();
	let resolved = schema;
	while (refs?.[resolved] !== undefined) {
		if (visited.has(resolved))
			throw new Error("Circular geometry schema reference");
		visited.add(resolved);
		resolved = refs[resolved];
	}
	let marker: unknown;
	try {
		marker = JSON.parse(resolved);
	} catch {
		throw new Error("Invalid geometry subtype marker");
	}
	if (
		!isObject(marker) ||
		Object.keys(marker).length !== 2 ||
		marker.$id !== GEOMETRY_SCHEMA_ID ||
		!GEOMETRY_KINDS.includes(marker["x-geometry"] as GeometryKind)
	) {
		throw new Error("Invalid geometry subtype marker");
	}
	return marker["x-geometry"] as GeometryKind;
}

/** Direction is output to input, including when a wire is dragged backwards. */
export function geometrySchemasCompatible(
	outputSchema?: string | null,
	inputSchema?: string | null,
	refs?: Record<string, string>,
): boolean {
	try {
		const output = geometryKindFromSchema(outputSchema, refs);
		const input = geometryKindFromSchema(inputSchema, refs);
		return input === null || input === output;
	} catch {
		return false;
	}
}

function ensureJsonProfile(
	value: unknown,
	depth = 0,
	budget = { members: 0 },
): void {
	if (++budget.members > MAX_GEOMETRY_MEMBERS)
		throw new Error("Geometry exceeds the member limit");
	if (depth > MAX_GEOMETRY_DEPTH)
		throw new Error("Geometry exceeds the nesting limit");
	if (typeof value === "number" && !Number.isFinite(value)) {
		throw new Error("Geometry numbers must be finite");
	}
	if (Array.isArray(value)) {
		for (const item of value) ensureJsonProfile(item, depth + 1, budget);
	} else if (isObject(value)) {
		for (const item of Object.values(value))
			ensureJsonProfile(item, depth + 1, budget);
	} else if (
		value !== null &&
		typeof value !== "string" &&
		typeof value !== "boolean" &&
		typeof value !== "number"
	) {
		throw new Error("Geometry must contain JSON values");
	}
}

/** Validates and normalizes polygon winding while retaining foreign members. */
export function normalizeGeometryValue(
	value: unknown,
	options: {
		schema?: string | null;
		refs?: Record<string, string>;
		valueType?: IValueType;
		allowUnset?: boolean;
	} = {},
): unknown {
	const kind = geometryKindFromSchema(options.schema, options.refs);
	if (value == null && options.allowUnset) return null;
	if (value == null) throw new Error("A geometry value is required");
	ensureJsonProfile(value);
	if (
		new TextEncoder().encode(JSON.stringify(value)).length >
		MAX_GEOMETRY_JSON_BYTES
	) {
		throw new Error("Geometry exceeds the JSON size limit");
	}
	let positions = 0;
	const array = (item: unknown, path: string): unknown[] => {
		if (!Array.isArray(item)) throw new Error(`${path} must be an array`);
		return item;
	};
	const position = (item: unknown, path: string, count = true): number[] => {
		const coords = array(item, path);
		if (
			coords.length !== 2 ||
			coords.some((n) => typeof n !== "number" || !Number.isFinite(n))
		)
			throw new Error(
				`${path} must contain two finite numbers: longitude, latitude`,
			);
		const [lon, lat] = coords as number[];
		if (lon < -180 || lon > 180 || lat < -90 || lat > 90) {
			throw new Error(`${path} is outside WGS 84 longitude/latitude bounds`);
		}
		if (count && ++positions > MAX_GEOMETRY_POSITIONS)
			throw new Error("Geometry exceeds the position limit");
		return [lon, lat];
	};
	const line = (item: unknown, path: string, minimum = 2): number[][] => {
		const coords = array(item, path);
		if (coords.length < minimum)
			throw new Error(`${path} needs at least ${minimum} positions`);
		return coords.map((p, i) => position(p, `${path}[${i}]`));
	};
	const polygon = (item: unknown, path: string): number[][][] => {
		const rings = array(item, path);
		if (rings.length === 0) throw new Error(`${path} needs an exterior ring`);
		return rings.map((item, i) => {
			const ring = line(item, `${path}[${i}]`, 4);
			const first = ring[0];
			const last = ring[ring.length - 1];
			if (first[0] !== last[0] || first[1] !== last[1])
				throw new Error(`${path}[${i}] must be closed`);
			let area = 0;
			for (let j = 1; j < ring.length; j++) {
				area += ring[j - 1][0] * ring[j][1] - ring[j][0] * ring[j - 1][1];
			}
			return (i === 0 && area < 0) || (i > 0 && area > 0)
				? ring.reverse()
				: ring;
		});
	};
	const geometry = (
		item: unknown,
		path: string,
		subtype: GeometryKind | null,
	): Record<string, unknown> => {
		if (
			!isObject(item) ||
			!GEOMETRY_KINDS.includes(item.type as GeometryKind)
		) {
			throw new Error(
				`${path} must be a GeoJSON geometry object; extract Feature wrappers first`,
			);
		}
		if (subtype && subtype !== item.type)
			throw new Error(`${path} must be ${subtype}, received ${item.type}`);
		if ("crs" in item)
			throw new Error(
				`${path}: alternate CRS declarations are unsupported; use WGS 84 longitude, latitude`,
			);
		if ("bbox" in item) {
			const bbox = array(item.bbox, `${path}.bbox`);
			if (bbox.length !== 4)
				throw new Error(`${path}.bbox must contain four coordinates`);
			position(bbox.slice(0, 2), `${path}.bbox southwest`, false);
			position(bbox.slice(2, 4), `${path}.bbox northeast`, false);
			if ((bbox[1] as number) > (bbox[3] as number))
				throw new Error(`${path}.bbox south must not exceed north`);
		}
		if (item.type === "GeometryCollection" && "coordinates" in item)
			throw new Error(`${path}: GeometryCollection uses geometries`);
		if (item.type !== "GeometryCollection" && "geometries" in item)
			throw new Error(
				`${path}: only GeometryCollection may contain geometries`,
			);
		const result = { ...item };
		const coords = item.coordinates;
		switch (item.type) {
			case "Point":
				result.coordinates = position(coords, `${path}.coordinates`);
				break;
			case "LineString":
				result.coordinates = line(coords, `${path}.coordinates`);
				break;
			case "Polygon":
				result.coordinates = polygon(coords, `${path}.coordinates`);
				break;
			case "MultiPoint":
				result.coordinates = array(coords, `${path}.coordinates`).map((p, i) =>
					position(p, `${path}.coordinates[${i}]`),
				);
				break;
			case "MultiLineString":
				result.coordinates = array(coords, `${path}.coordinates`).map((p, i) =>
					line(p, `${path}.coordinates[${i}]`),
				);
				break;
			case "MultiPolygon":
				result.coordinates = array(coords, `${path}.coordinates`).map((p, i) =>
					polygon(p, `${path}.coordinates[${i}]`),
				);
				break;
			case "GeometryCollection":
				result.geometries = array(item.geometries, `${path}.geometries`).map(
					(g, i) => geometry(g, `${path}.geometries[${i}]`, null),
				);
				break;
		}
		return result;
	};
	switch (options.valueType ?? IValueType.Normal) {
		case IValueType.Array:
		case IValueType.HashSet: {
			const items = array(value, "Geometry container").map((g, i) =>
				geometry(g, `[${i}]`, kind),
			);
			if (options.valueType === IValueType.HashSet) {
				const seen = new Set<string>();
				return items.filter((item) => {
					const key = JSON.stringify(item, (_key, value) =>
						isObject(value)
							? Object.fromEntries(
									Object.entries(value).sort(([a], [b]) => a.localeCompare(b)),
								)
							: value,
					);
					if (seen.has(key)) return false;
					seen.add(key);
					return true;
				});
			}
			return items;
		}
		case IValueType.HashMap:
			if (!isObject(value))
				throw new Error("Geometry map must be an object with string keys");
			return Object.fromEntries(
				Object.entries(value).map(([key, g]) => [
					key,
					geometry(g, `[${JSON.stringify(key)}]`, kind),
				]),
			);
		default:
			return geometry(value, "Geometry", kind);
	}
}

export function parseGeometryText(
	text: string,
	options: Parameters<typeof normalizeGeometryValue>[1] = {},
): unknown {
	if (new TextEncoder().encode(text).length > MAX_GEOMETRY_JSON_BYTES) {
		throw new Error("Geometry exceeds the JSON size limit");
	}
	return normalizeGeometryValue(
		text.trim() === "" ? null : JSON.parse(text),
		options,
	);
}
