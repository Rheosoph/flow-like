import { looksLikeTemporalName } from "../../../../lib/date";
import { resolveStorageFile } from "../../../../lib/storage-file";
import { looksLikeUserColumnName } from "../../../../lib/user-display";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { accountIdFromValue } from "../../../../state/backend-state/user-state";

export type ColumnKind =
	| "number"
	| "temporal"
	| "boolean"
	| "json"
	| "user"
	| "file"
	| "text";

export function classifyColumn(column: QueryColumn): ColumnKind {
	const type = column.type_name.toLowerCase();
	if (/bool/.test(type)) return "boolean";
	if (/date|time|timestamp|instant|duration|interval/.test(type))
		return "temporal";
	if (/int|float|double|decimal|numeric|number|real|serial/.test(type)) {
		// An integer column named `created_at` holds an instant; the declared type
		// is all the SQL layer knows, so the name is the only remaining signal.
		return looksLikeTemporalName(column.name) ? "temporal" : "number";
	}
	if (/json|struct|list|array|map|object|record/.test(type)) return "json";
	// A `created_by` or `user_sub` column holds a person, not a string.
	return looksLikeUserColumnName(column.name) ? "user" : "text";
}

/** How many rows are enough to tell a column of people from a column of words. */
const VALUE_SAMPLE = 100;

/**
 * The kind a column reads as in this particular result set.
 *
 * The user and file kinds can only be settled here: a name is a promise, not
 * proof, so a `created_by` that holds job names stays text rather than putting a
 * person icon over a column where no cell will ever resolve, and a stored path is
 * a file only once a value proves it points into this app's storage.
 */
export function classifyResultColumn(
	column: QueryColumn,
	rows: readonly Record<string, unknown>[],
	appId?: string,
): ColumnKind {
	const kind = classifyColumn(column);
	if (rows.length === 0) return kind;
	const sample = rows.slice(0, VALUE_SAMPLE);

	if (kind === "user") {
		return sample.some((row) => accountIdFromValue(row[column.name]))
			? "user"
			: "text";
	}

	if (
		kind === "text" &&
		appId &&
		sample.some((row) =>
			resolveStorageFile(column.name, row[column.name], appId),
		)
	) {
		return "file";
	}

	return kind;
}

export function isNumericColumn(column: QueryColumn): boolean {
	return classifyColumn(column) === "number";
}

export function isNullish(value: unknown): boolean {
	return value === null || value === undefined;
}

const numberFormat = new Intl.NumberFormat(undefined, {
	maximumFractionDigits: 6,
});

export function formatNumber(value: unknown): string {
	const numeric = typeof value === "number" ? value : Number(value);
	return Number.isFinite(numeric)
		? numberFormat.format(numeric)
		: String(value);
}

export function cellToString(value: unknown): string {
	if (isNullish(value)) return "";
	if (typeof value === "object") return JSON.stringify(value);
	return String(value);
}

export function csvField(value: unknown): string {
	const text = cellToString(value);
	return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function toCsv(
	columns: readonly QueryColumn[],
	rows: readonly Record<string, unknown>[],
): string {
	const header = columns.map((column) => csvField(column.name)).join(",");
	const body = rows
		.map((row) => columns.map((column) => csvField(row[column.name])).join(","))
		.join("\n");
	return `${header}\n${body}`;
}

export function toMarkdownTable(
	columns: readonly QueryColumn[],
	rows: readonly Record<string, unknown>[],
): string {
	const esc = (value: unknown) =>
		cellToString(value).replace(/\\/g, "\\\\").replace(/\|/g, "\\|");
	const header = `| ${columns.map((column) => esc(column.name)).join(" | ")} |`;
	const divider = `| ${columns.map(() => "---").join(" | ")} |`;
	const body = rows
		.map(
			(row) =>
				`| ${columns.map((column) => esc(row[column.name])).join(" | ")} |`,
		)
		.join("\n");
	return `${header}\n${divider}\n${body}`;
}
