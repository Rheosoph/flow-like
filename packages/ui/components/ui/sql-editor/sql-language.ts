import type { Monaco } from "@monaco-editor/react";
import type {
	QueryColumn,
	SavedQueryKind,
} from "../../../state/backend-state/query-state";

export const SQL_DIAGNOSTIC_OWNER = "flow-like-sql";

export interface SqlCatalogTable {
	name: string;
	scope?: "project" | "user";
	columns?: QueryColumn[];
}

export interface SqlCatalogView {
	name: string;
	columns?: QueryColumn[];
}

export interface SqlCatalog {
	tables: SqlCatalogTable[];
	views: SqlCatalogView[];
	/** Declared `$param` names (without the leading `$`). */
	params: string[];
}

const EMPTY_CATALOG: SqlCatalog = { tables: [], views: [], params: [] };

const SQL_KEYWORDS = [
	"SELECT",
	"FROM",
	"WHERE",
	"GROUP BY",
	"ORDER BY",
	"HAVING",
	"LIMIT",
	"OFFSET",
	"JOIN",
	"LEFT JOIN",
	"RIGHT JOIN",
	"INNER JOIN",
	"FULL JOIN",
	"ON",
	"AS",
	"AND",
	"OR",
	"NOT",
	"IN",
	"IS NULL",
	"IS NOT NULL",
	"LIKE",
	"BETWEEN",
	"DISTINCT",
	"COUNT",
	"SUM",
	"AVG",
	"MIN",
	"MAX",
	"CASE",
	"WHEN",
	"THEN",
	"ELSE",
	"END",
	"WITH",
	"UNION",
	"UNION ALL",
	"ASC",
	"DESC",
	"CAST",
	"COALESCE",
];

/** Clause keywords that can never be a table alias. */
const CLAUSE_KEYWORDS = new Set([
	"where",
	"join",
	"inner",
	"left",
	"right",
	"full",
	"outer",
	"on",
	"group",
	"order",
	"limit",
	"offset",
	"having",
	"union",
	"using",
	"cross",
	"as",
]);

/**
 * Replaces string-literal and comment contents with spaces so identifier and
 * parameter scanning never triggers inside them. Mirrors the FlowScript editor's
 * `maskLiterals`, adapted to SQL (single-quoted strings, `--` and block comments).
 */
export function maskSqlLiterals(text: string): string {
	let out = "";
	let state: "code" | "string" | "line-comment" | "block-comment" = "code";
	let i = 0;
	while (i < text.length) {
		const ch = text[i];
		const next = text[i + 1];
		if (state === "code") {
			if (ch === "'") {
				out += "'";
				state = "string";
			} else if (ch === "-" && next === "-") {
				out += "  ";
				i += 2;
				state = "line-comment";
				continue;
			} else if (ch === "/" && next === "*") {
				out += "  ";
				i += 2;
				state = "block-comment";
				continue;
			} else {
				out += ch;
			}
		} else if (state === "string") {
			if (ch === "'" && next === "'") {
				out += "  ";
				i += 2;
				continue;
			}
			if (ch === "'") {
				out += "'";
				state = "code";
			} else {
				out += ch === "\n" ? "\n" : " ";
			}
		} else if (state === "line-comment") {
			if (ch === "\n") {
				out += "\n";
				state = "code";
			} else {
				out += " ";
			}
		} else {
			if (ch === "*" && next === "/") {
				out += "  ";
				i += 2;
				state = "code";
				continue;
			}
			out += ch === "\n" ? "\n" : " ";
		}
		i += 1;
	}
	return out;
}

/** Extracts declared `$name` parameters from SQL, ignoring strings and comments. */
export function extractParams(sql: string): string[] {
	const masked = maskSqlLiterals(sql);
	const names = new Set<string>();
	const regex = /\$([A-Za-z_]\w*)/g;
	let match: RegExpExecArray | null = regex.exec(masked);
	while (match) {
		names.add(match[1]);
		match = regex.exec(masked);
	}
	return [...names];
}

/** Table/view names referenced in FROM/JOIN position — used to lazily load columns. */
export function extractReferencedTables(sql: string): string[] {
	const masked = maskSqlLiterals(sql);
	const names = new Set<string>();
	const regex = /\b(?:from|join|into|update)\s+([A-Za-z_]\w*)/gi;
	let match: RegExpExecArray | null = regex.exec(masked);
	while (match) {
		names.add(match[1]);
		match = regex.exec(masked);
	}
	return [...names];
}

/**
 * Recommends whether a query should be saved as a reusable View or a Stored
 * Query, from its shape. Deliberately conservative — it only pre-selects.
 */
export function recommendQueryKind(
	sql: string,
	params: string[],
): { kind: SavedQueryKind; reason: string } {
	const masked = maskSqlLiterals(sql).trim();
	if (params.length > 0) {
		return {
			kind: "query",
			reason: "Takes parameters — save as a Stored Query to run with inputs.",
		};
	}
	if (/\bwith\b/i.test(masked)) {
		return {
			kind: "query",
			reason: "Uses CTEs — save as a Stored Query.",
		};
	}
	const selectCount = (masked.match(/\bselect\b/gi) ?? []).length;
	if (selectCount === 1 && /^select\b/i.test(masked)) {
		return {
			kind: "view",
			reason:
				"Reusable single SELECT — save as a View so other queries can FROM it.",
		};
	}
	return { kind: "query", reason: "Save as a Stored Query." };
}

function buildAliasMap(maskedSql: string): Map<string, string> {
	const aliases = new Map<string, string>();
	const regex =
		/\b(?:from|join)\s+([A-Za-z_]\w*)(?:\s+(?:as\s+)?([A-Za-z_]\w*))?/gi;
	let match: RegExpExecArray | null = regex.exec(maskedSql);
	while (match) {
		const table = match[1];
		const alias = match[2];
		if (alias && !CLAUSE_KEYWORDS.has(alias.toLowerCase())) {
			aliases.set(alias.toLowerCase(), table);
		}
		match = regex.exec(maskedSql);
	}
	return aliases;
}

function resolveColumns(
	reference: string,
	sql: string,
	catalog: SqlCatalog,
): QueryColumn[] {
	const aliases = buildAliasMap(maskSqlLiterals(sql));
	const target = (
		aliases.get(reference.toLowerCase()) ?? reference
	).toLowerCase();
	const table =
		catalog.tables.find((item) => item.name.toLowerCase() === target) ??
		catalog.views.find((item) => item.name.toLowerCase() === target);
	return table?.columns ?? [];
}

// Module-scoped so completion/diagnostics register exactly once against the
// global `sql` language id. The mounted editor pushes the live catalog here.
let providersRegistered = false;
let activeCatalog: SqlCatalog = EMPTY_CATALOG;

export function setActiveSqlCatalog(catalog: SqlCatalog): void {
	activeCatalog = catalog;
}

export function ensureSqlProviders(monaco: Monaco): void {
	if (providersRegistered) return;
	providersRegistered = true;

	monaco.languages.registerCompletionItemProvider("sql", {
		triggerCharacters: [".", " ", "$", "("],
		provideCompletionItems(model, position) {
			const kinds = monaco.languages.CompletionItemKind;
			const word = model.getWordUntilPosition(position);
			const range = {
				startLineNumber: position.lineNumber,
				endLineNumber: position.lineNumber,
				startColumn: word.startColumn,
				endColumn: word.endColumn,
			};
			const linePrefix = model.getValueInRange({
				startLineNumber: position.lineNumber,
				startColumn: 1,
				endLineNumber: position.lineNumber,
				endColumn: position.column,
			});
			const catalog = activeCatalog;
			const items: Array<Record<string, unknown>> = [];

			if (/\$(\w*)$/.test(linePrefix)) {
				const names = new Set([
					...catalog.params,
					...extractParams(model.getValue()),
				]);
				for (const name of names) {
					items.push({
						label: `$${name}`,
						kind: kinds.Variable,
						insertText: name,
						detail: "parameter",
						range,
					});
				}
				return { suggestions: items as never };
			}

			const dotMatch = linePrefix.match(/([A-Za-z_]\w*)\.(\w*)$/);
			if (dotMatch) {
				for (const column of resolveColumns(
					dotMatch[1],
					model.getValue(),
					catalog,
				)) {
					items.push({
						label: column.name,
						kind: kinds.Field,
						insertText: column.name,
						detail: column.type_name,
						range,
					});
				}
				return { suggestions: items as never };
			}

			if (/\b(from|join|into|update|table)\s+[\w.]*$/i.test(linePrefix)) {
				for (const table of catalog.tables) {
					items.push({
						label: table.name,
						kind: kinds.Struct,
						insertText: table.name,
						detail: table.scope === "user" ? "user table" : "table",
						range,
					});
				}
				for (const view of catalog.views) {
					items.push({
						label: view.name,
						kind: kinds.Interface,
						insertText: view.name,
						detail: "view",
						range,
					});
				}
				return { suggestions: items as never };
			}

			for (const keyword of SQL_KEYWORDS) {
				items.push({
					label: keyword,
					kind: kinds.Keyword,
					insertText: keyword,
					range,
				});
			}
			for (const table of catalog.tables) {
				items.push({
					label: table.name,
					kind: kinds.Struct,
					insertText: table.name,
					detail: "table",
					range,
				});
			}
			for (const view of catalog.views) {
				items.push({
					label: view.name,
					kind: kinds.Interface,
					insertText: view.name,
					detail: "view",
					range,
				});
			}
			const seenColumns = new Set<string>();
			for (const source of [...catalog.tables, ...catalog.views]) {
				for (const column of source.columns ?? []) {
					if (seenColumns.has(column.name)) continue;
					seenColumns.add(column.name);
					items.push({
						label: column.name,
						kind: kinds.Field,
						insertText: column.name,
						detail: column.type_name,
						range,
					});
				}
			}
			return { suggestions: items as never };
		},
	});
}

function offsetToLineColumn(
	text: string,
	offset: number,
): { line: number; column: number } {
	let line = 1;
	let column = 1;
	for (let i = 0; i < offset && i < text.length; i += 1) {
		if (text[i] === "\n") {
			line += 1;
			column = 1;
		} else {
			column += 1;
		}
	}
	return { line, column };
}

/**
 * Conservative client-side diagnostics: flags FROM/JOIN targets that are not a
 * known table, view, or CTE — but only once the catalog has loaded, to avoid
 * false positives while table names are still being fetched.
 */
export function computeSqlDiagnostics(
	monaco: Monaco,
	sql: string,
	catalog: SqlCatalog,
): { markers: Array<Record<string, unknown>> } {
	const markers: Array<Record<string, unknown>> = [];
	if (!catalog || catalog.tables.length === 0) return { markers };

	const masked = maskSqlLiterals(sql);
	const cteNames = new Set<string>();
	const cteRegex = /\b([A-Za-z_]\w*)\s+as\s*\(/gi;
	let cte: RegExpExecArray | null = cteRegex.exec(masked);
	while (cte) {
		cteNames.add(cte[1].toLowerCase());
		cte = cteRegex.exec(masked);
	}

	const known = new Set<string>([
		...catalog.tables.map((table) => table.name.toLowerCase()),
		...catalog.views.map((view) => view.name.toLowerCase()),
		...cteNames,
	]);

	const regex = /\b(from|join)\s+([A-Za-z_]\w*)/gi;
	let match: RegExpExecArray | null = regex.exec(masked);
	while (match) {
		const name = match[2];
		if (!known.has(name.toLowerCase())) {
			const offset = match.index + match[0].length - name.length;
			const start = offsetToLineColumn(sql, offset);
			markers.push({
				severity: monaco.MarkerSeverity.Warning,
				message: `Unknown table or view '${name}'`,
				startLineNumber: start.line,
				startColumn: start.column,
				endLineNumber: start.line,
				endColumn: start.column + name.length,
			});
		}
		match = regex.exec(masked);
	}
	return { markers };
}
