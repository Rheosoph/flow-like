import {
	APP_CATEGORIES,
	EXPLORE_SEARCH_LIMITS,
	type ExploreCategoryFacet,
	type ExplorePermissionFacet,
	type ExploreSearchQuery,
	type ExploreSearchSort,
	type ExploreSearchType,
	type ExploreTypeFilter,
	WASM_PACKAGE_CATEGORIES,
} from "./explore-types";

export const EXPLORE_PATH = "/store/explore";
export const EXPLORE_SEARCH_PATH = "/store/explore/search";

/** The Browse page's URL state: everything in ExploreSearchQuery except viewer and paging fields. */
export type ExploreBrowseParams = Pick<
	ExploreSearchQuery,
	| "q"
	| "type"
	| "categories"
	| "price"
	| "verified"
	| "permissions"
	| "sort"
	| "collection"
>;

const SEARCH_TYPES: readonly ExploreSearchType[] = [
	"all",
	"apps",
	"packages",
	"collections",
];
const SEARCH_SORTS: readonly ExploreSearchSort[] = [
	"best",
	"newest",
	"rating",
	"installs",
	"name",
	"updated",
];
const PERMISSIONS: readonly ExplorePermissionFacet[] = [
	"none",
	"network",
	"models",
	"storage",
];
const CATEGORY_FACETS: ReadonlySet<string> = new Set<ExploreCategoryFacet>([
	...APP_CATEGORIES.map((category): ExploreCategoryFacet => `app:${category}`),
	...WASM_PACKAGE_CATEGORIES.map(
		(category): ExploreCategoryFacet => `package:${category}`,
	),
]);

const LEGACY_APP_SORTS: ReadonlyMap<string, ExploreSearchSort> = new Map([
	["popular", "best"],
	["rated", "rating"],
	["newest", "newest"],
	["updated", "updated"],
]);

function oneOf<T extends string>(
	allowed: readonly T[],
	value: string | null | undefined,
): T | undefined {
	return allowed.find((candidate) => candidate === value);
}

function commaList(value: string | null): string[] {
	if (!value) return [];
	return [
		...new Set(
			value
				.split(",")
				.map((entry) => entry.trim())
				.filter(Boolean),
		),
	];
}

function encodeParam(value: string): string {
	return encodeURIComponent(value).replace(/%3A/gi, ":").replace(/%2C/gi, ",");
}

function joinList(
	values: readonly string[] | undefined,
	max: number,
): string | undefined {
	if (!values) return undefined;
	return (
		[...new Set(values.filter(Boolean))].slice(0, max).join(",") || undefined
	);
}

/** Trimmed and cut to the server's `q` limit on a code point boundary; `undefined` when empty. */
export function exploreSearchText(
	value: string | null | undefined,
): string | undefined {
	const trimmed = value?.trim();
	if (!trimmed) return undefined;
	return (
		Array.from(trimmed).slice(0, EXPLORE_SEARCH_LIMITS.q).join("").trim() ||
		undefined
	);
}

/** Browse filters as query entries, capped to the server limits: lists comma-joined, empty values dropped. */
export function browseParamEntries(
	params: ExploreBrowseParams,
): [string, string][] {
	const entries: [string, string | undefined][] = [
		["type", params.type],
		["q", exploreSearchText(params.q)],
		[
			"categories",
			joinList(params.categories, EXPLORE_SEARCH_LIMITS.categories),
		],
		["price", params.price],
		["verified", params.verified ? "true" : undefined],
		[
			"permissions",
			joinList(params.permissions, EXPLORE_SEARCH_LIMITS.permissions),
		],
		["sort", params.sort],
		["collection", params.collection?.trim() || undefined],
	];
	return entries.filter(
		(entry): entry is [string, string] => entry[1] !== undefined,
	);
}

export function isExploreCategoryFacet(
	value: string,
): value is ExploreCategoryFacet {
	return CATEGORY_FACETS.has(value);
}

export function exploreTypeFromParam(
	value: string | null | undefined,
): ExploreTypeFilter {
	return value === "apps" || value === "packages" ? value : "all";
}

/** The curated landing; `type` preselects the All · Apps · Packages filter. */
export function exploreHref({
	type,
}: { type?: ExploreTypeFilter } = {}): string {
	return type && type !== "all" ? `${EXPLORE_PATH}?type=${type}` : EXPLORE_PATH;
}

export function parseBrowseParams(
	params: Pick<URLSearchParams, "get">,
): ExploreBrowseParams {
	const result: ExploreBrowseParams = {};
	const q = exploreSearchText(params.get("q"));
	if (q) result.q = q;
	const type = oneOf(SEARCH_TYPES, params.get("type"));
	if (type) result.type = type;
	const categories = commaList(params.get("categories"))
		.filter(isExploreCategoryFacet)
		.slice(0, EXPLORE_SEARCH_LIMITS.categories);
	if (categories.length) result.categories = categories;
	const price = params.get("price");
	if (price === "free" || price === "paid") result.price = price;
	if (params.get("verified") === "true") result.verified = true;
	const permissions = commaList(params.get("permissions")).filter(
		(entry): entry is ExplorePermissionFacet =>
			oneOf(PERMISSIONS, entry) !== undefined,
	);
	if (permissions.length) result.permissions = permissions;
	const sort = oneOf(SEARCH_SORTS, params.get("sort"));
	if (sort) result.sort = sort;
	const collection = params.get("collection")?.trim();
	if (collection) result.collection = collection;
	return result;
}

/** Query string (no leading `?`) with lists comma-joined and `:`/`,` left readable. */
export function serializeBrowseParams(params: ExploreBrowseParams): string {
	return browseParamEntries(params)
		.map(([key, value]) => `${key}=${encodeParam(value)}`)
		.join("&");
}

export function exploreSearchHref(params: ExploreBrowseParams = {}): string {
	const query = serializeBrowseParams(params);
	return query ? `${EXPLORE_SEARCH_PATH}?${query}` : EXPLORE_SEARCH_PATH;
}

/** Target for the retired `/store/explore/apps?q&category&sort`: unfiltered → the curated landing, filtered → Browse. */
export function legacyAppsExploreTarget(
	params: Pick<URLSearchParams, "get">,
): string {
	const q = exploreSearchText(params.get("q"));
	const rawCategory = params.get("category");
	const category = rawCategory
		? APP_CATEGORIES.find((candidate) => candidate === rawCategory)
		: undefined;
	const rawSort = params.get("sort");
	const sort = rawSort ? LEGACY_APP_SORTS.get(rawSort) : undefined;
	if (!q && !category && (!sort || rawSort === "popular")) {
		return exploreHref({ type: "apps" });
	}
	return exploreSearchHref({
		type: "apps",
		q,
		categories: category ? [`app:${category}`] : undefined,
		sort,
	});
}
