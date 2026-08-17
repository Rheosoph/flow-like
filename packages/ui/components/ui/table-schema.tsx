"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { IIndexType } from "../../state/backend-state/db-state";
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

export interface IndexTypeOption {
	value: string;
	label: string;
	type: IIndexType;
}

export const INDEX_TYPE_OPTIONS: IndexTypeOption[] = [
	{ value: "auto", label: "Auto", type: IIndexType.Auto },
	{ value: "btree", label: "BTree", type: IIndexType.BTree },
	{ value: "bitmap", label: "Bitmap", type: IIndexType.Bitmap },
	{ value: "fulltext", label: "Full Text", type: IIndexType.FullText },
	{ value: "labellist", label: "Label List", type: IIndexType.LabelList },
];

export function indexTypeEnum(value: string): IIndexType {
	return (
		INDEX_TYPE_OPTIONS.find((option) => option.value === value)?.type ??
		IIndexType.Auto
	);
}

// --- Shared validation ------------------------------------------------------

const RESERVED_COLUMNS = new Set(["_rowid", "_distance", "_relevance_score"]);
const COLUMN_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;
const TABLE_NAME_PATTERN = /^[A-Za-z0-9._-]+$/;

export function validateColumnName(name: string): string | null {
	const trimmed = name.trim();
	if (!trimmed) return i18next.t('nameIsRequired', 'Name is required');
	if (trimmed.length > 128) return i18next.t('max128Characters', 'Max 128 characters');
	if (!COLUMN_NAME_PATTERN.test(trimmed))
		return i18next.t('lettersNumbersAndUnderscoresOnlyCannotStartWithANumber', 'Letters, numbers and underscores only; cannot start with a number');
	if (RESERVED_COLUMNS.has(trimmed.toLowerCase()))
		return i18next.t('trimmedIsReservedByLancedb', '"{{trimmed}}" is reserved by LanceDB', { trimmed });
	return null;
}

export function validateTableName(name: string): string | null {
	const trimmed = name.trim();
	if (!trimmed) return i18next.t('nameIsRequired', 'Name is required');
	if (trimmed.length > 256) return i18next.t('max256Characters', 'Max 256 characters');
	if (!TABLE_NAME_PATTERN.test(trimmed))
		return i18next.t('lettersNumbersDotDashAndUnderscoreOnly', 'Letters, numbers, dot, dash and underscore only');
	if (trimmed.includes("..")) return i18next.t('cannotContain', 'Cannot contain \'..\'');
	if (/^__.*__$/.test(trimmed)) return "This name is reserved";
	return null;
}

// --- Shared field controls --------------------------------------------------

export function ColumnTypeSelect({
	value,
	onChange,
	groups = CREATE_COLUMN_TYPE_GROUPS,
	disabled,
	id,
	className,
}: Readonly<{
	value: string;
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
							<SelectItem key={option.value} value={option.value}>
								{option.label}
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
				<SelectItem value="nullable">{t('nullable', 'Nullable')}</SelectItem>
				<SelectItem value="required">{t('required', 'Required')}</SelectItem>
			</SelectContent>
		</Select>
	);
}

export function IndexTypeSelect({
	value,
	onChange,
	disabled,
	className,
}: Readonly<{
	value: string;
	onChange: (value: string) => void;
	disabled?: boolean;
	className?: string;
}>) {
	return (
		<Select value={value} onValueChange={onChange} disabled={disabled}>
			<SelectTrigger className={className}>
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{INDEX_TYPE_OPTIONS.map((option) => (
					<SelectItem key={option.value} value={option.value}>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}
