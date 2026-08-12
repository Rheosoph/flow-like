import MiniSearch, { type SearchOptions } from "minisearch";

export interface SearchIndexOptions<T> {
	/**
	 * Indexed fields. Supports dotted paths (`manifest.name`, `meta.en.description`)
	 * and array/object values, which are flattened to text.
	 */
	readonly fields: readonly string[];
	/** Per-field weights, keyed exactly like `fields`. */
	readonly boost?: Record<string, number>;
	/**
	 * Extra searchable text that is not reachable through a field path. Must be
	 * a pure function of the item — `useSearchIndex` does not rebuild when state
	 * captured by this closure changes, so derive such text into the items.
	 */
	readonly extract?: (item: T) => string | undefined | null;
	/** `AND` (default) narrows as the user types, `OR` widens. */
	readonly combineWith?: "AND" | "OR";
	/** Fuzzy edit distance as a fraction of term length. Defaults to 0.2. */
	readonly fuzzy?: number | false;
	/** Prefix matching for as-you-type search. Defaults to true. */
	readonly prefix?: boolean;
}

export interface SearchIndex<T> {
	/** Ranked matches for `query`; every item, in order, for an empty query. */
	readonly search: (query: string) => T[];
	readonly isEmpty: boolean;
}

const EXTRA_FIELD = "__extra";
const ID_FIELD = "__idx";

function readPath(source: unknown, path: string): unknown {
	if (!path.includes("."))
		return (source as Record<string, unknown> | null | undefined)?.[path];
	let current: unknown = source;
	for (const segment of path.split(".")) {
		if (current === null || typeof current !== "object") return undefined;
		current = (current as Record<string, unknown>)[segment];
	}
	return current;
}

function toText(value: unknown): string {
	if (value === null || value === undefined) return "";
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean")
		return String(value);
	if (value instanceof Date) return value.toISOString();
	if (Array.isArray(value)) return value.map(toText).filter(Boolean).join(" ");
	if (typeof value === "object")
		return Object.values(value as Record<string, unknown>)
			.map(toText)
			.filter(Boolean)
			.join(" ");
	return "";
}

/**
 * Split on whitespace and punctuation, then additionally on camelCase
 * boundaries so `chartKit` stays reachable via "kit".
 */
export function tokenizeSearchText(text: string): string[] {
	const tokens = text.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
	const camelParts: string[] = [];
	for (const token of tokens) {
		const parts = token.split(/(?<=[a-z0-9])(?=[A-Z])/);
		if (parts.length > 1) camelParts.push(...parts);
	}
	return camelParts.length > 0 ? [...tokens, ...camelParts] : tokens;
}

/**
 * MiniSearch-backed in-memory index over a client-side collection.
 *
 * Built synchronously (no `addAllAsync` re-index races), keyed by array
 * position (no id requirement, no duplicate-id rejections) and resolved back
 * to the original objects, so callers keep their own item types.
 */
export function buildSearchIndex<T>(
	items: readonly T[] | undefined,
	options: SearchIndexOptions<T>,
): SearchIndex<T> {
	const {
		fields,
		boost,
		extract,
		combineWith = "AND",
		fuzzy = 0.2,
		prefix = true,
	} = options;

	const list = items ?? [];
	const searchOptions: SearchOptions = {
		prefix,
		combineWith,
		...(fuzzy === false ? {} : { fuzzy }),
		...(boost ? { boost } : {}),
	};

	const index = new MiniSearch<{ __idx: number; item: T }>({
		idField: ID_FIELD,
		fields: extract ? [...fields, EXTRA_FIELD] : [...fields],
		extractField: (document, field) => {
			if (field === ID_FIELD) return String(document.__idx);
			if (field === EXTRA_FIELD) return extract?.(document.item) ?? "";
			return toText(readPath(document.item, field));
		},
		tokenize: tokenizeSearchText,
		processTerm: (term) => term.toLowerCase(),
		searchOptions,
	});

	index.addAll(list.map((item, idx) => ({ __idx: idx, item })));

	return {
		isEmpty: list.length === 0,
		search: (query: string) => {
			const trimmed = query.trim();
			if (!trimmed) return [...list];
			return index
				.search(trimmed)
				.map((result) => list[Number(result.id)])
				.filter((item): item is T => item !== undefined);
		},
	};
}
