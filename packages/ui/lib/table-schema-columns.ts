import { isGeometryMetadata } from "./geometry-columns";

export interface TableColumnSummary {
	name: string;
	/** `create_table` vocabulary where one applies, otherwise the Arrow type. */
	type: string;
	nullable: boolean;
	primary_key?: true;
	vector_size?: number;
}

const PRIMARY_KEY_POSITION = "lance-schema:unenforced-primary-key:position";
const LEGACY_PRIMARY_KEY = "lance-schema:unenforced-primary-key";
const MAX_LISTED_KEYS = 20;

const record = (value: unknown): Record<string, unknown> | undefined =>
	typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;

function arrowTypeArgument(variant: string, arg: unknown): string {
	const field = record(arg);
	if (!field) return String(arg ?? "None");
	if (!("data_type" in field)) return arrowTypeName(field);
	const type = arrowTypeName(field.data_type);
	return variant === "Struct" ? `${String(field.name)}: ${type}` : type;
}

/** Render a serde-serialized Arrow `DataType` (`"Utf8"`, `{ Timestamp: [...] }`, …). */
function arrowTypeName(dataType: unknown): string {
	if (typeof dataType === "string") return dataType;
	const [variant, payload] = Object.entries(record(dataType) ?? {})[0] ?? [];
	if (!variant) return JSON.stringify(dataType) ?? "unknown";
	const args = Array.isArray(payload) ? payload : [payload];
	const rendered = args.map((arg) => arrowTypeArgument(variant, arg));
	return `${variant}(${rendered.join(", ")})`;
}

const SCALAR_COLUMN_TYPES = new Map([
	["Utf8", "string"],
	["LargeUtf8", "string"],
	["Utf8View", "string"],
	["Boolean", "boolean"],
	["Binary", "binary"],
	["LargeBinary", "binary"],
	["BinaryView", "binary"],
	["Date32", "date32"],
]);

function scalarColumnType(dataType: string): string {
	if (/^(U?Int(8|16|32|64)|Float(16|32|64))$/.test(dataType))
		return dataType.toLowerCase();
	return SCALAR_COLUMN_TYPES.get(dataType) ?? dataType;
}

function columnType(
	dataType: unknown,
	metadata: Record<string, unknown> | undefined,
): { type: string; vector_size?: number } {
	if (isGeometryMetadata(metadata)) return { type: "geometry" };
	if (typeof dataType === "string") return { type: scalarColumnType(dataType) };
	const timestamp = record(dataType)?.Timestamp;
	if (
		Array.isArray(timestamp) &&
		timestamp[0] === "Millisecond" &&
		timestamp[1] === "UTC"
	) {
		return { type: "timestamp:ms:UTC" };
	}
	const vector = record(dataType)?.FixedSizeList;
	if (
		Array.isArray(vector) &&
		record(vector[0])?.data_type === "Float32" &&
		typeof vector[1] === "number"
	) {
		return { type: "vector", vector_size: vector[1] };
	}
	return { type: arrowTypeName(dataType) };
}

function isPrimaryKey(metadata: Record<string, unknown> | undefined): boolean {
	if (!metadata) return false;
	if (metadata[PRIMARY_KEY_POSITION] !== undefined) return true;
	const legacy = metadata[LEGACY_PRIMARY_KEY];
	return legacy !== undefined && String(legacy).toLowerCase() !== "false";
}

/** Summarize a serde-serialized Arrow schema (`{ fields: [...] }`) in `create_table` terms. */
export function summarizeTableColumns(schema: unknown): TableColumnSummary[] {
	const fields = record(schema)?.fields;
	if (!Array.isArray(fields)) return [];
	return fields.flatMap((entry) => {
		const field = record(entry);
		if (typeof field?.name !== "string") return [];
		const metadata = record(field.metadata);
		return [
			{
				name: field.name,
				...columnType(field.data_type, metadata),
				nullable: field.nullable !== false,
				...(isPrimaryKey(metadata) ? { primary_key: true as const } : {}),
			},
		];
	});
}

function quotedList(values: readonly string[]): string {
	const listed = values.slice(0, MAX_LISTED_KEYS).map((value) => `'${value}'`);
	return values.length > MAX_LISTED_KEYS
		? `${listed.join(", ")} and ${values.length - MAX_LISTED_KEYS} more`
		: listed.join(", ");
}

/** Storage drops keys that are not columns of an existing table, so refuse them up front. */
export function assertRowKeysAreColumns(
	tableName: string,
	rows: readonly unknown[],
	columns: readonly string[],
): void {
	const known = new Set(columns);
	const unknown = new Set<string>();
	for (const row of rows) {
		for (const key of Object.keys(record(row) ?? {})) {
			if (!known.has(key)) unknown.add(key);
		}
	}
	if (unknown.size === 0) return;
	throw new Error(
		`Table '${tableName}' has no column ${quotedList([...unknown])}; its columns are ${columns.join(", ")}. Add them with add_column or rename the keys.`,
	);
}
