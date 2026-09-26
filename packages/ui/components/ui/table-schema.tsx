"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { UNSUPPORTED_GEOARROW_INDEX_HELP } from "../../lib/geoarrow-index";
import { IIndexType, parseIndexType } from "../../state/backend-state/db-state";
import { Select } from "./select";
import {
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
	SelectValue,
} from "./select";

// --- Shared type vocabulary -------------------------------------------------

export interface ColumnTypeOption {
	value: string;
	label: string;
}

export interface ColumnTypeGroup {
	label: string;
	options: ColumnTypeOption[];
}

/** Full type vocabulary supported by `createTable` (physical schema). */
export const CREATE_COLUMN_TYPE_GROUPS: ColumnTypeGroup[] = [
	{ label: "Text", options: [{ value: "string", label: "String" }] },
	{ label: "Boolean", options: [{ value: "boolean", label: "Boolean" }] },
	{
		label: "Integer",
		options: [
			{ value: "int32", label: "Integer (32-bit)" },
			{ value: "int64", label: "Big Integer (64-bit)" },
			{ value: "int16", label: "Small Integer (16-bit)" },
			{ value: "int8", label: "Tiny Integer (8-bit)" },
			{ value: "uint32", label: "Unsigned Integer (32-bit)" },
			{ value: "uint64", label: "Unsigned Big Integer (64-bit)" },
			{ value: "uint16", label: "Unsigned Small Integer (16-bit)" },
			{ value: "uint8", label: "Unsigned Tiny Integer (8-bit)" },
		],
	},
	{
		label: "Float",
		options: [
			{ value: "float32", label: "Float (32-bit)" },
			{ value: "float64", label: "Double (64-bit)" },
		],
	},
	{
		label: "Temporal",
		options: [
			{ value: "date32", label: "Date" },
			{ value: "timestamp", label: "Timestamp" },
		],
	},
	{
		label: "Geometry",
		options: [{ value: "geometry", label: "Geometry (WGS 84)" }],
	},
	{ label: "Binary", options: [{ value: "binary", label: "Binary" }] },
	{
		label: "Vector",
		options: [{ value: "vector", label: "Vector (Float32)" }],
	},
];

/**
 * Types that can be added to an *existing* table. LanceDB only lets us add a
 * column through a typed SQL expression, so this is the subset that maps to a
 * `CAST(... AS <type>)` expression (no vector / unsigned / narrow int columns).
 */
export const EDIT_COLUMN_TYPE_GROUPS: ColumnTypeGroup[] = [
	{ label: "Text", options: [{ value: "string", label: "String" }] },
	{ label: "Boolean", options: [{ value: "boolean", label: "Boolean" }] },
	{
		label: "Integer",
		options: [
			{ value: "int32", label: "Integer" },
			{ value: "int64", label: "Big Integer" },
		],
	},
	{
		label: "Float",
		options: [
			{ value: "float32", label: "Float" },
			{ value: "float64", label: "Double" },
		],
	},
	{
		label: "Temporal",
		options: [
			{ value: "date32", label: "Date" },
			{ value: "timestamp", label: "Timestamp" },
		],
	},
	{ label: "Binary", options: [{ value: "binary", label: "Binary" }] },
];

const SQL_CAST_TYPES: Record<string, string> = {
	string: "STRING",
	boolean: "BOOLEAN",
	int32: "INT",
	int64: "BIGINT",
	float32: "FLOAT",
	float64: "DOUBLE",
	binary: "BINARY",
	date32: "DATE",
	timestamp: "TIMESTAMP",
};

const QUOTED_SQL_TYPES = new Set(["STRING", "DATE", "TIMESTAMP", "BINARY"]);

export function addColumnSqlType(type: string): string | undefined {
	return SQL_CAST_TYPES[type];
}

const INTEGER_LITERAL = /^-?\d+$/;
const FLOAT_LITERAL = /^-?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i;
const DATE_LITERAL = /^\d{4}-\d{2}-\d{2}$/;
const TIMESTAMP_LITERAL =
	/^\d{4}-\d{2}-\d{2}([ T]\d{2}:\d{2}(:\d{2}(\.\d+)?)?)?$/;

/**
 * Numeric and boolean defaults are spliced into the SQL expression unquoted,
 * so they must be real literals before they reach `buildAddColumnExpression`.
 */
export function validateColumnDefault(
	type: string,
	value: string,
): string | null {
	const trimmed = value.trim();
	if (!trimmed) return null;
	if ((type === "int32" || type === "int64") && !INTEGER_LITERAL.test(trimmed))
		return i18next.t("enterAWholeNumber", "Enter a whole number");
	if (
		(type === "float32" || type === "float64") &&
		!FLOAT_LITERAL.test(trimmed)
	)
		return i18next.t("enterANumber", "Enter a number");
	if (type === "boolean" && trimmed !== "true" && trimmed !== "false")
		return i18next.t("enterTrueOrFalse", "Enter true or false");
	if (type === "date32" && !DATE_LITERAL.test(trimmed))
		return i18next.t("useTheFormatYyyyMmDd", "Use the format YYYY-MM-DD");
	if (type === "timestamp" && !TIMESTAMP_LITERAL.test(trimmed))
		return i18next.t(
			"useTheFormatYyyyMmDdHhMmSs",
			"Use the format YYYY-MM-DD HH:MM:SS",
		);
	return null;
}

/**
 * Build the typed SQL expression LanceDB needs to add a column. An empty
 * default yields a typed NULL; a provided default becomes a typed literal.
 */
export function buildAddColumnExpression(
	type: string,
	defaultValue?: string,
): string | null {
	const sqlType = SQL_CAST_TYPES[type];
	if (!sqlType) return null;
	const trimmed = defaultValue?.trim() ?? "";
	if (!trimmed) return `CAST(NULL AS ${sqlType})`;
	const literal = QUOTED_SQL_TYPES.has(sqlType)
		? `'${trimmed.replace(/'/g, "''")}'`
		: trimmed;
	return `CAST(${literal} AS ${sqlType})`;
}

// --- Table key (Lance's unenforced primary key) ------------------------------

const KEY_POSITION_METADATA = "lance-schema:unenforced-primary-key:position";
const LEGACY_KEY_METADATA = "lance-schema:unenforced-primary-key";
const MAX_KEY_POSITION = 4_294_967_295;
/** The types Lance's inserted-key conflict filter hashes; it skips any other type. */
const KEY_ARROW_TYPES = new Set([
	"Int32",
	"Int64",
	"UInt32",
	"UInt64",
	"Utf8",
	"LargeUtf8",
	"Binary",
	"LargeBinary",
]);
/** `geometry` is Binary too, but a shape is no row identity. */
const KEY_COLUMN_TYPES = new Set([
	"string",
	"int32",
	"int64",
	"uint32",
	"uint64",
	"binary",
]);

/** Whether Arrow field metadata marks the column as the table key, read as Lance reads it. */
export function isKeyFieldMetadata(metadata: unknown): boolean {
	if (!metadata || typeof metadata !== "object") return false;
	const record = metadata as Record<string, unknown>;
	const position = String(record[KEY_POSITION_METADATA] ?? "");
	if (/^\+?\d+$/.test(position) && Number(position) <= MAX_KEY_POSITION)
		return true;
	const legacy = record[LEGACY_KEY_METADATA];
	return typeof legacy === "string" && /^(true|1|yes)$/i.test(legacy);
}

/**
 * Whether an existing top-level Arrow field can become the key. It must be
 * required; a nullable column has to be made required first, which Lance only
 * allows while it holds no NULLs.
 */
export function canArrowFieldBeKey(field: {
	data_type?: unknown;
	nullable?: boolean;
}): boolean {
	if (field.nullable !== false) return false;
	return (
		typeof field.data_type === "string" && KEY_ARROW_TYPES.has(field.data_type)
	);
}

/** Whether a designer column type can be the key of a new table. */
export function isKeyColumnType(type: string): boolean {
	return KEY_COLUMN_TYPES.has(type);
}

export interface IndexTypeOption {
	value: string;
	label: string;
	type: IIndexType;
	category: "auto" | "scalar" | "vector";
	columnKinds?: readonly string[];
	description?: string;
}

export const INDEX_TYPE_OPTIONS: IndexTypeOption[] = [
	{ value: "auto", label: "Auto", type: IIndexType.Auto, category: "auto" },
	{
		value: "btree",
		label: "BTree",
		type: IIndexType.BTree,
		category: "scalar",
		columnKinds: ["string", "number", "date", "boolean", "binary"],
	},
	{
		value: "bitmap",
		label: "Bitmap",
		type: IIndexType.Bitmap,
		category: "scalar",
		columnKinds: ["string", "number", "date", "boolean", "binary"],
	},
	{
		value: "fulltext",
		label: "Full Text",
		type: IIndexType.FullText,
		category: "scalar",
		columnKinds: ["string"],
	},
	{
		value: "labellist",
		label: "Label List",
		type: IIndexType.LabelList,
		category: "scalar",
		columnKinds: ["array"],
	},
	{
		value: "fm",
		label: "FM (substring)",
		type: IIndexType.Fm,
		category: "scalar",
		columnKinds: ["string", "binary"],
	},
	{
		value: "ngram",
		label: "N-gram (substring)",
		type: IIndexType.NGram,
		category: "scalar",
		columnKinds: ["string"],
		description:
			"Speeds up text substring filters. Requires a managed Lance table; LanceDB Cloud is not supported.",
	},
	{
		value: "zonemap",
		label: "Zone Map (ranges)",
		type: IIndexType.ZoneMap,
		category: "scalar",
		columnKinds: ["string", "number", "date"],
		description:
			"Skips data blocks outside string, numeric, date, or timestamp ranges. Works best when nearby rows have nearby values. Requires a managed Lance table; LanceDB Cloud is not supported.",
	},
	{
		value: "bloomfilter",
		label: "Bloom Filter (equality)",
		type: IIndexType.BloomFilter,
		category: "scalar",
		columnKinds: ["string", "number", "date", "binary"],
		description:
			"Skips data blocks that cannot contain an equality match. Requires a managed Lance table; LanceDB Cloud is not supported.",
	},
	{
		value: "rtree",
		label: "R-Tree (GeoArrow)",
		type: IIndexType.RTree,
		category: "scalar",
		columnKinds: ["geometry"],
		description:
			"Indexes spatial bounds for GeoArrow WKB/WKT or separate Float64 coordinate fields. Requires a managed Lance table; LanceDB Cloud is not supported.",
	},
	{
		value: "vector",
		label: "Vector (cosine IVF-PQ)",
		type: IIndexType.Vector,
		category: "vector",
	},
	{
		value: "ivfflat",
		label: "IVF Flat",
		type: IIndexType.IvfFlat,
		category: "vector",
	},
	{
		value: "ivfpq",
		label: "IVF PQ",
		type: IIndexType.IvfPq,
		category: "vector",
	},
	{
		value: "ivfsq",
		label: "IVF SQ",
		type: IIndexType.IvfSq,
		category: "vector",
	},
	{
		value: "ivfrq",
		label: "IVF RQ",
		type: IIndexType.IvfRq,
		category: "vector",
	},
	{
		value: "ivfhnswflat",
		label: "IVF HNSW Flat",
		type: IIndexType.IvfHnswFlat,
		category: "vector",
	},
	{
		value: "ivfhnswpq",
		label: "IVF HNSW PQ",
		type: IIndexType.IvfHnswPq,
		category: "vector",
	},
	{
		value: "ivfhnswsq",
		label: "IVF HNSW SQ",
		type: IIndexType.IvfHnswSq,
		category: "vector",
	},
];

export function indexTypeEnum(value: string): IIndexType {
	return parseIndexType(value);
}

/** Match the table designer's physical types to the explorer's column kinds. */
export function getIndexTypeOptions(
	columnType?: string,
	category?: "scalar" | "vector",
): IndexTypeOption[] {
	let kind = columnType?.toLowerCase();
	if (/^(u?int\d+|float\d+|double|decimal.*)$/.test(kind ?? ""))
		kind = "number";
	if (/^(date\d*|timestamp.*|time\d*)$/.test(kind ?? "")) kind = "date";
	if (kind === "bool") kind = "boolean";
	if (kind === "struct") kind = "object";
	if (kind === "list") kind = "array";
	return INDEX_TYPE_OPTIONS.filter((option) => {
		if (option.category === "auto") return true;
		if (option.type === IIndexType.RTree && kind !== "geometry") return false;
		if (category && option.category !== category) return false;
		if (!kind || kind === "unknown") return true;
		if (option.category === "vector") return kind === "vector";
		if (kind === "vector") return false;
		return !option.columnKinds || option.columnKinds.includes(kind);
	});
}

// --- Shared validation ------------------------------------------------------

const RESERVED_COLUMNS = new Set(["_rowid", "_distance", "_relevance_score"]);
const COLUMN_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;
const TABLE_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;

export function validateColumnName(name: string): string | null {
	const trimmed = name.trim();
	if (!trimmed) return i18next.t("nameIsRequired", "Name is required");
	if (trimmed.length > 128)
		return i18next.t("max128Characters", "Max 128 characters");
	if (!COLUMN_NAME_PATTERN.test(trimmed))
		return i18next.t(
			"lettersNumbersAndUnderscoresOnlyCannotStartWithANumber",
			"Letters, numbers and underscores only; cannot start with a number",
		);
	if (RESERVED_COLUMNS.has(trimmed.toLowerCase()))
		return i18next.t(
			"trimmedIsReservedByLancedb",
			'"{{trimmed}}" is reserved by LanceDB',
			{ trimmed },
		);
	return null;
}

export function validateTableName(name: string): string | null {
	const trimmed = name.trim();
	if (!trimmed) return i18next.t("nameIsRequired", "Name is required");
	if (trimmed.length > 256)
		return i18next.t("max256Characters", "Max 256 characters");
	if (!TABLE_NAME_PATTERN.test(trimmed))
		return i18next.t(
			"lettersNumbersDotDashAndUnderscoreOnly",
			"Letters, numbers, dot, dash and underscore only",
		);
	if (trimmed.includes(".."))
		return i18next.t("cannotContain", "Cannot contain '..'");
	if (/^__.*__$/.test(trimmed)) return "This name is reserved";
	return null;
}

// --- Shared field controls --------------------------------------------------

export function ColumnTypeSelect({
	value,
	geometryEnabled = false,
	onChange,
	groups = CREATE_COLUMN_TYPE_GROUPS,
	disabled,
	id,
	className,
}: Readonly<{
	value: string;
	geometryEnabled?: boolean;
	onChange: (value: string) => void;
	groups?: ColumnTypeGroup[];
	disabled?: boolean;
	id?: string;
	className?: string;
}>) {
	return (
		<Select value={value} onValueChange={onChange} disabled={disabled}>
			<SelectTrigger id={id} className={className}>
				<SelectValue placeholder="Type" />
			</SelectTrigger>
			<SelectContent>
				{groups.map((group) => (
					<SelectGroup key={group.label}>
						<SelectLabel>{group.label}</SelectLabel>
						{group.options.map((option) => (
							<SelectItem
								key={option.value}
								value={option.value}
								disabled={option.value === "geometry" && !geometryEnabled}
							>
								{option.value === "geometry"
									? i18next.t("settings:geometryWgs84", "Geometry (WGS 84)")
									: option.label}
							</SelectItem>
						))}
					</SelectGroup>
				))}
			</SelectContent>
		</Select>
	);
}

export function NullableSelect({
	nullable,
	onChange,
	disabled,
	id,
	className,
}: Readonly<{
	nullable: boolean;
	onChange: (nullable: boolean) => void;
	disabled?: boolean;
	id?: string;
	className?: string;
}>) {
	const { t } = useTranslation("common");
	return (
		<Select
			value={nullable ? "nullable" : "required"}
			onValueChange={(value) => onChange(value === "nullable")}
			disabled={disabled}
		>
			<SelectTrigger id={id} className={className}>
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				<SelectItem value="nullable">{t("nullable", "Nullable")}</SelectItem>
				<SelectItem value="required">{t("required", "Required")}</SelectItem>
			</SelectContent>
		</Select>
	);
}

export function IndexTypeSelect({
	value,
	onChange,
	disabled,
	className,
	category,
	columnType,
}: Readonly<{
	value: string;
	onChange: (value: string) => void;
	disabled?: boolean;
	className?: string;
	category?: "scalar" | "vector";
	columnType?: string;
}>) {
	const { t } = useTranslation("common");
	const options = getIndexTypeOptions(columnType, category);
	return (
		<Select value={value} onValueChange={onChange} disabled={disabled}>
			<SelectTrigger
				className={className}
				aria-label={t("indexType", "Index type")}
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{options.map((option) => (
					<SelectItem key={option.value} value={option.value}>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

export function IndexTypeHelp({
	value,
	columnType,
}: Readonly<{ value: string; columnType?: string }>) {
	if (columnType === "unsupported-geometry") {
		return (
			<p className="basis-full text-xs text-muted-foreground">
				{UNSUPPORTED_GEOARROW_INDEX_HELP}
			</p>
		);
	}
	const description = INDEX_TYPE_OPTIONS.find(
		(option) => option.value === value,
	)?.description;
	if (!description) return null;
	return (
		<p className="basis-full text-xs text-muted-foreground">{description}</p>
	);
}
