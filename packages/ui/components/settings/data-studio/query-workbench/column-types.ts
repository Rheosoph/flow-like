import type { QueryColumn } from "../../../../state/backend-state/query-state";

export type ColumnKind = "number" | "temporal" | "boolean" | "json" | "text";

export function classifyColumn(column: QueryColumn): ColumnKind {
	const type = column.type_name.toLowerCase();
	if (/bool/.test(type)) return "boolean";
	if (/date|time|timestamp|instant|duration|interval/.test(type))
		return "temporal";
	if (/int|float|double|decimal|numeric|number|real|serial/.test(type))
		return "number";
	if (/json|struct|list|array|map|object|record/.test(type)) return "json";
	return "text";
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
	const esc = (value: unknown) => cellToString(value).replace(/\|/g, "\\|");
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
