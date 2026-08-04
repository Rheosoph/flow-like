"use client";

import { useMemo, useRef } from "react";
import {
	type SearchIndex,
	type SearchIndexOptions,
	buildSearchIndex,
} from "../lib/search-index";

/**
 * Memoized MiniSearch index over a client-side collection. The index is only
 * rebuilt when the item list or the field configuration changes, so inline
 * `fields`/`boost`/`extract` literals are safe at the call site.
 */
export function useSearchIndex<T>(
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

	const extractRef = useRef(extract);
	extractRef.current = extract;

	const fieldKey = fields.join("|");
	const boostKey = boost ? JSON.stringify(boost) : "";
	const hasExtract = Boolean(extract);

	return useMemo(
		() =>
			buildSearchIndex(items, {
				fields: fieldKey ? fieldKey.split("|") : [],
				boost: boostKey ? JSON.parse(boostKey) : undefined,
				extract: hasExtract
					? (item: T) => extractRef.current?.(item) ?? ""
					: undefined,
				combineWith,
				fuzzy,
				prefix,
			}),
		[items, fieldKey, boostKey, hasExtract, combineWith, fuzzy, prefix],
	);
}

/**
 * Index `items` and return ranked matches for `query`. An empty query yields
 * the original list, unchanged and in order.
 */
export function useSearch<T>(
	items: readonly T[] | undefined,
	query: string,
	options: SearchIndexOptions<T>,
): T[] {
	const index = useSearchIndex(items, options);
	return useMemo(() => index.search(query), [index, query]);
}
