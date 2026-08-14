export type Bundles = Record<string, Record<string, Record<string, unknown>>>;

const KEY_SEPARATOR = "\u0000";

export interface LocaleConfig {
	sourceLanguage: string;
	defaultNamespace: string;
	namespaces: string[];
	languages: string[];
}

/** `{ a: { b: "x" } }` → `{ "a.b": "x" }`. Only strings survive; i18next values are strings. */
export function flatten(
	tree: Record<string, unknown>,
	prefix = "",
): Record<string, string> {
	const out: Record<string, string> = {};
	for (const [key, value] of Object.entries(tree)) {
		const path = prefix ? `${prefix}${KEY_SEPARATOR}${key}` : key;
		if (value && typeof value === "object" && !Array.isArray(value)) {
			Object.assign(out, flatten(value as Record<string, unknown>, path));
		} else if (typeof value === "string") {
			out[path] = value;
		}
	}
	return out;
}

/** Inverse of `flatten`, with keys sorted so writes produce stable diffs. */
export function unflatten(
	flat: Record<string, string>,
): Record<string, unknown> {
	const out: Record<string, unknown> = {};
	for (const path of Object.keys(flat).sort()) {
		const segments = path.split(KEY_SEPARATOR);
		let cursor = out;
		segments.forEach((segment, index) => {
			if (index === segments.length - 1) {
				setOwn(cursor, segment, flat[path]);
				return;
			}
			const existing = Object.hasOwn(cursor, segment)
				? cursor[segment]
				: undefined;
			if (!isRecord(existing)) {
				setOwn(cursor, segment, {});
			}
			cursor = cursor[segment] as Record<string, unknown>;
		});
	}
	return out;
}

/** Human-readable i18next key path; tree segments are joined with dots. */
export function displayKey(key: string): string {
	return key.split(KEY_SEPARATOR).join(".");
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Define rather than assign so locale keys such as `__proto__` stay data. */
function setOwn(
	target: Record<string, unknown>,
	key: string,
	value: unknown,
): void {
	Object.defineProperty(target, key, {
		value,
		enumerable: true,
		configurable: true,
		writable: true,
	});
}

/**
 * Runtime-significant i18next tokens. Interpolations and nesting references
 * carry values, while numeric tags identify components rendered by
 * react-i18next's `Trans`. Dropping any of them can break the rendered result.
 */
export function placeholders(value: string): string[] {
	return [
		...value.matchAll(
			/\{\{\s*([^}]+?)\s*\}\}|\$t\(\s*([^)]+?)\s*\)|<\s*(\/?)\s*(\d+)\s*(\/?)\s*>/g,
		),
	]
		.map((match) => {
			if (match[1] !== undefined) return `{{${match[1].trim()}}}`;
			if (match[2] !== undefined) return `$t(${match[2].trim()})`;
			return `<${match[3] ? "/" : ""}${match[4]}${match[5] ? "/" : ""}>`;
		})
		.sort();
}

/** Multiset subtraction: losing one of two repeated tokens is still a loss. */
export function missingPlaceholders(source: string, value: string): string[] {
	const available = new Map<string, number>();
	for (const token of placeholders(value)) {
		available.set(token, (available.get(token) ?? 0) + 1);
	}

	return placeholders(source).filter((token) => {
		const remaining = available.get(token) ?? 0;
		if (remaining === 0) return true;
		available.set(token, remaining - 1);
		return false;
	});
}

export type KeyStatus =
	| "translated"
	| "missing"
	| "copied"
	| "orphan"
	| "broken";

export interface KeyRow {
	key: string;
	namespace: string;
	source: string;
	value: string;
	status: KeyStatus;
	/** Placeholders present in the source but absent from the translation. */
	lostPlaceholders: string[];
	/** Percent longer (or shorter) than the source string. */
	lengthDelta: number;
}

export function statusOf(source: string | undefined, value: string): KeyStatus {
	if (source === undefined) return "orphan";
	if (!value.trim()) return "missing";
	const lost = missingPlaceholders(source, value);
	if (lost.length) return "broken";
	if (value === source && source.trim()) return "copied";
	return "translated";
}

export function buildRows(
	bundles: Bundles,
	namespaces: string[],
	sourceLanguage: string,
	language: string,
): KeyRow[] {
	const rows: KeyRow[] = [];

	for (const namespace of namespaces) {
		const source = flatten(bundles[sourceLanguage]?.[namespace] ?? {});
		const target = flatten(bundles[language]?.[namespace] ?? {});
		const keys = new Set([...Object.keys(source), ...Object.keys(target)]);

		for (const key of [...keys].sort()) {
			const sourceValue = source[key];
			const value = target[key] ?? "";
			const lost =
				sourceValue === undefined
					? []
					: missingPlaceholders(sourceValue, value);
			rows.push({
				key,
				namespace,
				source: sourceValue ?? "",
				value,
				status: statusOf(sourceValue, value),
				lostPlaceholders: value.trim() ? lost : [],
				lengthDelta:
					sourceValue && value
						? Math.round(
								((value.length - sourceValue.length) / sourceValue.length) *
									100,
							)
						: 0,
			});
		}
	}

	return rows;
}

export interface Coverage {
	total: number;
	/** Valid translated values, including deliberate copies of the source. */
	complete: number;
	translated: number;
	missing: number;
	problems: number;
	percent: number;
}

export function coverageOf(rows: KeyRow[]): Coverage {
	// Orphans are problems to remove, not source work to complete. Including
	// them in the denominator makes coverage fall when a source key is deleted.
	const sourceRows = rows.filter((row) => row.status !== "orphan");
	const total = sourceRows.length;
	const translated = sourceRows.filter(
		(row) => row.status === "translated",
	).length;
	// A string copied from the source is still rendered, so it counts towards
	// coverage — it is flagged separately as something worth a second look.
	const copied = sourceRows.filter((row) => row.status === "copied").length;
	const complete = translated + copied;
	const missing = sourceRows.filter((row) => row.status === "missing").length;
	const problems = rows.filter(
		(row) =>
			row.status === "broken" ||
			row.status === "copied" ||
			row.status === "orphan",
	).length;
	return {
		total,
		complete,
		translated,
		missing,
		problems,
		percent: total ? Math.round((complete / total) * 100) : 0,
	};
}

/** Display name for a locale code, in the reader's own language. */
export function languageLabel(code: string, displayIn = "en"): string {
	try {
		return (
			new Intl.DisplayNames([displayIn], { type: "language" }).of(code) ?? code
		);
	} catch {
		return code;
	}
}
