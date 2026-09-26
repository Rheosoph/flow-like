import type { IDatabaseSchemaField } from "../state/backend-state/db-state";
import {
	GEOMETRY_KINDS,
	type GeometryKind,
	normalizeGeometryValue,
} from "./geometry";

export const GEOJSON_IMPORT_BATCH_MAX_ROWS = 250;
export const GEOJSON_IMPORT_BATCH_MAX_BYTES = 1_000_000;

const MAX_TABLE_COLUMNS = 256;
const MAX_COLUMN_NAME_LENGTH = 128;
const COLUMN_NAME_SUFFIX_RESERVE = 8;
const RESERVED_COLUMN_NAMES = [
	"_rowid",
	"_distance",
	"_relevance_score",
	"__proto__",
];
const VALID_COLUMN_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const RFC3339_DATE_TIME =
	/^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|[+-](\d{2}):(\d{2}))$/;
const WGS84_CRS_NAMES = new Set([
	"crs84",
	"ogc:crs84",
	"urn:ogc:def:crs:ogc:1.3:crs84",
	"urn:ogc:def:crs:ogc::crs84",
	"http://www.opengis.net/def/crs/ogc/1.3/crs84",
	"epsg:4326",
	"urn:ogc:def:crs:epsg::4326",
	"urn:ogc:def:crs:epsg:4326",
	"http://www.opengis.net/def/crs/epsg/0/4326",
]);

export interface GeoJsonImportOptions {
	geometryColumn: string;
	keyColumn: string;
}

export interface GeoJsonImportColumn {
	name: string;
	type: string;
	source_property?: string;
}

export interface GeoJsonImportSkip {
	/** Zero-based position of the feature in the file. */
	feature_index: number;
	reason: string;
}

export interface GeoJsonImportPlan {
	/** `create_table` fields: the key, the geometry, then one column per property. */
	fields: IDatabaseSchemaField[];
	rows: Record<string, unknown>[];
	/** Zero-based feature position of each row, to name the feature an insert failed at. */
	rowFeatureIndexes: number[];
	columns: GeoJsonImportColumn[];
	skipped: GeoJsonImportSkip[];
	warnings: string[];
	featureCount: number;
}

type ValueKind = "boolean" | "number" | "string" | "json";

interface PropertyColumn {
	name: string;
	property: string;
	renamedFrom?: string;
	kinds: Set<ValueKind>;
	safeIntegers: boolean;
	timestamps: boolean;
	type: string;
}

interface AcceptedFeature {
	index: number;
	id?: string;
	properties: Record<string, unknown>;
	geometry: Record<string, unknown> | null;
	droppedOrdinates: boolean;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function describeJsonValue(value: unknown): string {
	if (value === null) return "null";
	if (Array.isArray(value)) return "an array";
	if (isRecord(value))
		return typeof value.type === "string"
			? `a '${value.type}' object`
			: "an object without a GeoJSON type";
	return `a ${typeof value}`;
}

/** `{ type: "name", properties: { name } }` or the 2008 `{ type: "EPSG", properties: { code } }`. */
function crsName(crs: unknown): string | undefined {
	if (!isRecord(crs) || !isRecord(crs.properties)) return undefined;
	const { name, code } = crs.properties;
	if (typeof name === "string") return name;
	if (code === undefined) return undefined;
	const authority = typeof crs.type === "string" ? crs.type : "EPSG";
	return `${authority}:${String(code)}`;
}

/** The declared CRS when it is not WGS 84 longitude/latitude; undefined when absent or WGS 84. */
function foreignCrs(crs: unknown): string | undefined {
	if (crs === undefined || crs === null) return undefined;
	const name = crsName(crs);
	if (name && WGS84_CRS_NAMES.has(name.trim().toLowerCase())) return undefined;
	return name ?? JSON.stringify(crs).slice(0, 200);
}

function geoJsonFeatures(parsed: unknown): unknown[] {
	if (Array.isArray(parsed)) return parsed;
	if (isRecord(parsed)) {
		const crs = foreignCrs(parsed.crs);
		if (crs) {
			throw new Error(
				`The GeoJSON declares crs '${crs}', but import_geojson only reads WGS 84 longitude/latitude (CRS84 / EPSG:4326). Reproject the file to WGS 84 and attach it again.`,
			);
		}
		if (parsed.type === "FeatureCollection") {
			if (!Array.isArray(parsed.features)) {
				throw new Error(
					`The GeoJSON FeatureCollection has no 'features' array (found ${describeJsonValue(parsed.features)}).`,
				);
			}
			return parsed.features;
		}
		if (parsed.type === "Feature") return [parsed];
	}
	throw new Error(
		`import_geojson expects a GeoJSON FeatureCollection, a Feature, or an array of Features; the file contains ${describeJsonValue(parsed)}.`,
	);
}

function flattenPositions(
	value: unknown,
	state: { dropped: boolean },
): unknown {
	if (!Array.isArray(value)) return value;
	if (value.every((item) => typeof item === "number")) {
		if (value.length <= 2) return value;
		state.dropped = true;
		return value.slice(0, 2);
	}
	return value.map((item) => flattenPositions(item, state));
}

/** Storage keeps neither bbox nor crs, and both would fail the 2D profile check after Z/M drop. */
function flattenGeometry(
	geometry: Record<string, unknown>,
	state: { dropped: boolean },
): Record<string, unknown> {
	const { bbox: _bbox, crs: _crs, ...flat } = geometry;
	if ("coordinates" in flat)
		flat.coordinates = flattenPositions(flat.coordinates, state);
	if (Array.isArray(flat.geometries)) {
		flat.geometries = flat.geometries.map((member) =>
			isRecord(member) ? flattenGeometry(member, state) : member,
		);
	}
	return flat;
}

function readGeometry(value: unknown): {
	geometry: Record<string, unknown> | null;
	dropped: boolean;
} {
	if (value === undefined || value === null)
		return { geometry: null, dropped: false };
	if (
		!isRecord(value) ||
		!GEOMETRY_KINDS.includes(value.type as GeometryKind)
	) {
		throw new Error(
			`geometry must be a GeoJSON geometry object or null, found ${describeJsonValue(value)}`,
		);
	}
	const crs = foreignCrs(value.crs);
	if (crs) {
		throw new Error(
			`geometry declares crs '${crs}'; only WGS 84 longitude/latitude is supported`,
		);
	}
	const state = { dropped: false };
	const flat = flattenGeometry(value, state);
	try {
		return {
			geometry: normalizeGeometryValue(flat) as Record<string, unknown>,
			dropped: state.dropped,
		};
	} catch (error) {
		throw new Error(`invalid geometry: ${errorMessage(error)}`);
	}
}

function readFeature(value: unknown, index: number): AcceptedFeature {
	if (!isRecord(value) || value.type !== "Feature") {
		throw new Error(
			`expected a GeoJSON Feature, found ${describeJsonValue(value)}`,
		);
	}
	const crs = foreignCrs(value.crs);
	if (crs) {
		throw new Error(
			`feature declares crs '${crs}'; only WGS 84 longitude/latitude is supported`,
		);
	}
	const { properties } = value;
	if (
		properties !== undefined &&
		properties !== null &&
		!isRecord(properties)
	) {
		throw new Error(
			`properties must be an object or null, found ${describeJsonValue(properties)}`,
		);
	}
	const { geometry, dropped } = readGeometry(value.geometry);
	const id =
		(typeof value.id === "string" && value.id.trim() !== "") ||
		(typeof value.id === "number" && Number.isFinite(value.id))
			? String(value.id)
			: undefined;
	return {
		index,
		id,
		properties: properties ?? {},
		geometry,
		droppedOrdinates: dropped,
	};
}

function assertColumnOption(option: string, value: string): void {
	if (
		value.length > MAX_COLUMN_NAME_LENGTH ||
		!VALID_COLUMN_NAME.test(value) ||
		RESERVED_COLUMN_NAMES.includes(value.toLowerCase())
	) {
		throw new Error(
			`import_geojson ${option} '${value}' is not a valid column name (ASCII letters, digits and underscores, not starting with a digit, at most ${MAX_COLUMN_NAME_LENGTH} characters, not ${RESERVED_COLUMN_NAMES.join("/")}).`,
		);
	}
}

/** `physicalSiteId` → `physical_site_id`, `addr:city` → `addr_city`, `2020 value` → `_2020_value`. */
export function geoJsonPropertyColumnName(property: string): string {
	const snake = property
		.normalize("NFKD")
		.replace(/\p{M}+/gu, "")
		.replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
		.replace(/([a-z0-9])([A-Z])/g, "$1_$2")
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "_")
		.replace(/^_+|_+$/g, "")
		.slice(0, MAX_COLUMN_NAME_LENGTH - COLUMN_NAME_SUFFIX_RESERVE)
		.replace(/_+$/, "");
	const name = snake || "property";
	return /^[0-9]/.test(name) ? `_${name}` : name;
}

export function isRfc3339DateTime(value: string): boolean {
	const match = RFC3339_DATE_TIME.exec(value);
	if (!match) return false;
	const [year, month, day, hour, minute, second] = match
		.slice(1, 7)
		.map(Number);
	const offsetHour = match[7] === undefined ? 0 : Number(match[7]);
	const offsetMinute = match[8] === undefined ? 0 : Number(match[8]);
	const daysInMonth = new Date(Date.UTC(year, month, 0)).getUTCDate();
	return (
		month >= 1 &&
		month <= 12 &&
		day >= 1 &&
		day <= daysInMonth &&
		hour <= 23 &&
		minute <= 59 &&
		second <= 59 &&
		offsetHour <= 23 &&
		offsetMinute <= 59
	);
}

function valueKind(value: unknown): ValueKind {
	switch (typeof value) {
		case "boolean":
			return "boolean";
		case "number":
			return "number";
		case "string":
			return "string";
		default:
			return "json";
	}
}

function inferColumnType(column: PropertyColumn): string {
	if (column.kinds.size !== 1) return "string";
	switch ([...column.kinds][0]) {
		case "boolean":
			return "boolean";
		case "number":
			return column.safeIntegers ? "int64" : "float64";
		case "string":
			return column.timestamps ? "timestamp:ms:UTC" : "string";
		default:
			return "string";
	}
}

function columnValue(value: unknown, type: string): unknown {
	if (value === undefined || value === null) return null;
	if (type !== "string" || typeof value === "string") return value;
	return typeof value === "object" ? JSON.stringify(value) : String(value);
}

function planPropertyColumns(
	features: readonly AcceptedFeature[],
	options: GeoJsonImportOptions,
): PropertyColumn[] {
	const taken = new Set(
		[options.keyColumn, options.geometryColumn, ...RESERVED_COLUMN_NAMES].map(
			(name) => name.toLowerCase(),
		),
	);
	const columns = new Map<string, PropertyColumn>();
	for (const feature of features) {
		for (const [property, value] of Object.entries(feature.properties)) {
			let column = columns.get(property);
			if (!column) {
				const base = geoJsonPropertyColumnName(property);
				let name = base;
				for (let suffix = 2; taken.has(name.toLowerCase()); suffix++) {
					name = `${base}_${suffix}`;
				}
				taken.add(name.toLowerCase());
				column = {
					name,
					property,
					...(name !== base ? { renamedFrom: base } : {}),
					kinds: new Set(),
					safeIntegers: true,
					timestamps: true,
					type: "string",
				};
				columns.set(property, column);
			}
			if (value === undefined || value === null) continue;
			column.kinds.add(valueKind(value));
			if (typeof value === "number")
				column.safeIntegers &&= Number.isSafeInteger(value);
			if (typeof value === "string")
				column.timestamps &&= isRfc3339DateTime(value);
		}
	}
	for (const column of columns.values()) column.type = inferColumnType(column);
	return [...columns.values()];
}

/** Explicit ids win: a duplicate or a generated `feature-<n>` never displaces a real id. */
function assignFeatureKeys(features: readonly AcceptedFeature[]): {
	keys: string[];
	duplicates: number;
	generated: number;
} {
	const taken = new Set(
		features.flatMap((feature) =>
			feature.id === undefined ? [] : [feature.id],
		),
	);
	const firstSeen = new Set<string>();
	const withSuffix = (base: string) => {
		for (let suffix = 2; ; suffix++) {
			const key = `${base}-${suffix}`;
			if (!taken.has(key)) {
				taken.add(key);
				return key;
			}
		}
	};
	const keys: string[] = new Array(features.length);
	let duplicates = 0;
	features.forEach((feature, position) => {
		if (feature.id === undefined) return;
		if (firstSeen.has(feature.id)) {
			duplicates++;
			keys[position] = withSuffix(feature.id);
		} else {
			firstSeen.add(feature.id);
			keys[position] = feature.id;
		}
	});
	let generated = 0;
	features.forEach((feature, position) => {
		if (feature.id !== undefined) return;
		generated++;
		const base = `feature-${feature.index + 1}`;
		if (taken.has(base)) {
			keys[position] = withSuffix(base);
		} else {
			taken.add(base);
			keys[position] = base;
		}
	});
	return { keys, duplicates, generated };
}

function planWarnings(
	features: readonly AcceptedFeature[],
	columns: readonly PropertyColumn[],
	keys: { duplicates: number; generated: number },
	options: GeoJsonImportOptions,
): string[] {
	const warnings: string[] = [];
	const flattened = features.filter(
		(feature) => feature.droppedOrdinates,
	).length;
	if (flattened > 0)
		warnings.push(
			`Dropped Z/M ordinates from ${flattened} feature geometr${flattened === 1 ? "y" : "ies"}; coordinates are stored as 2D longitude/latitude.`,
		);
	if (keys.generated > 0)
		warnings.push(
			`${keys.generated} of ${features.length} features had no id; their ${options.keyColumn} is feature-<n> (1-based position in the file).`,
		);
	if (keys.duplicates > 0)
		warnings.push(
			`${keys.duplicates} duplicate feature id(s) were suffixed with -<n> to keep ${options.keyColumn} unique.`,
		);
	const withoutGeometry = features.filter(
		(feature) => feature.geometry === null,
	).length;
	if (withoutGeometry > 0)
		warnings.push(
			`${withoutGeometry} feature(s) have a null geometry; ${options.geometryColumn} is null for them.`,
		);
	const mixed = columns.filter((column) => column.kinds.size > 1);
	if (mixed.length > 0)
		warnings.push(
			`Stored as text because their values mix types: ${mixed.map((column) => column.name).join(", ")}.`,
		);
	const nested = columns.filter((column) => column.kinds.has("json"));
	if (nested.length > 0)
		warnings.push(
			`Nested objects/arrays are stored as JSON text in: ${nested.map((column) => column.name).join(", ")}.`,
		);
	for (const column of columns) {
		if (column.renamedFrom)
			warnings.push(
				`Property '${column.property}' is stored as '${column.name}' because '${column.renamedFrom}' is already taken.`,
			);
	}
	return warnings;
}

/**
 * Plan a Data Studio import of GeoJSON features: one row per feature keyed by its id, the geometry
 * in one geometry column and each property in a snake_case column typed over every feature.
 * Invalid features are skipped with a reason instead of failing the import.
 */
export function planGeoJsonImport(
	parsed: unknown,
	options: GeoJsonImportOptions,
): GeoJsonImportPlan {
	assertColumnOption("key_column", options.keyColumn);
	assertColumnOption("geometry_column", options.geometryColumn);
	if (
		options.keyColumn.toLowerCase() === options.geometryColumn.toLowerCase()
	) {
		throw new Error(
			`import_geojson key_column and geometry_column must differ (both are '${options.keyColumn}').`,
		);
	}

	const values = geoJsonFeatures(parsed);
	const features: AcceptedFeature[] = [];
	const skipped: GeoJsonImportSkip[] = [];
	values.forEach((value, index) => {
		try {
			features.push(readFeature(value, index));
		} catch (error) {
			skipped.push({ feature_index: index, reason: errorMessage(error) });
		}
	});

	const propertyColumns = planPropertyColumns(features, options);
	const columnCount = propertyColumns.length + 2;
	if (columnCount > MAX_TABLE_COLUMNS) {
		throw new Error(
			`The GeoJSON properties need ${columnCount} columns, but a table holds at most ${MAX_TABLE_COLUMNS}. Remove unneeded properties from the file first.`,
		);
	}

	const keys = assignFeatureKeys(features);
	const rows = features.map((feature, position) => {
		const row: Record<string, unknown> = {
			[options.keyColumn]: keys.keys[position],
			[options.geometryColumn]: feature.geometry,
		};
		for (const column of propertyColumns) {
			row[column.name] = columnValue(
				Object.hasOwn(feature.properties, column.property)
					? feature.properties[column.property]
					: undefined,
				column.type,
			);
		}
		return row;
	});

	return {
		fields: [
			{
				name: options.keyColumn,
				type: "string",
				nullable: false,
				primary_key: true,
			},
			{ name: options.geometryColumn, type: "geometry", nullable: true },
			...propertyColumns.map((column) => ({
				name: column.name,
				type: column.type,
				nullable: true,
			})),
		],
		rows,
		rowFeatureIndexes: features.map((feature) => feature.index),
		columns: [
			{ name: options.keyColumn, type: "string" },
			{ name: options.geometryColumn, type: "geometry" },
			...propertyColumns.map((column) => ({
				name: column.name,
				type: column.type,
				source_property: column.property,
			})),
		],
		skipped,
		warnings: planWarnings(features, propertyColumns, keys, options),
		featureCount: values.length,
	};
}

/** Split rows into insert batches bounded by row count and serialized JSON size. */
export function batchGeoJsonRows<T>(
	rows: readonly T[],
	limits: { maxRows?: number; maxBytes?: number } = {},
): { start: number; rows: T[] }[] {
	const maxRows = limits.maxRows ?? GEOJSON_IMPORT_BATCH_MAX_ROWS;
	const maxBytes = limits.maxBytes ?? GEOJSON_IMPORT_BATCH_MAX_BYTES;
	const encoder = new TextEncoder();
	const batches: { start: number; rows: T[] }[] = [];
	let current: T[] = [];
	let start = 0;
	let bytes = 2;
	rows.forEach((row, index) => {
		const size = encoder.encode(JSON.stringify(row)).length + 1;
		if (
			current.length > 0 &&
			(current.length >= maxRows || bytes + size > maxBytes)
		) {
			batches.push({ start, rows: current });
			current = [];
			start = index;
			bytes = 2;
		}
		current.push(row);
		bytes += size;
	});
	if (current.length > 0) batches.push({ start, rows: current });
	return batches;
}
