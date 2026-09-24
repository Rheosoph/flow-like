import { exploreTypeFromParam } from "./explore-href";
import { APP_CATEGORIES } from "./explore-types";

/** On a legacy hub the Packages hub renders the old package list itself, so the redirect cannot loop. */
export const LEGACY_PACKAGES_HREF = "/store/packages?tab=explore";

type LegacyAppsSort = "newest" | "rated" | "updated";

const LEGACY_SORTS: ReadonlyMap<string, LegacyAppsSort> = new Map([
	["newest", "newest"],
	["updated", "updated"],
	["rated", "rated"],
	["rating", "rated"],
]);

function appCategoryOf(value: string | null | undefined) {
	return APP_CATEGORIES.find((category) => category === value);
}

/**
 * The query the old Explore apps page understands (`q`, `category`, `sort=newest|rated|updated`), read from either
 * vocabulary: Browse's (`categories=app:X`, `sort=rating`) or its own, so applying it twice changes nothing.
 * `type=apps|packages` stays: the old page ignores and preserves it, and the fallback reads it again once developer
 * mode is known. Everything else (package facets, collection) has no legacy equivalent and is dropped.
 */
export function legacyAppsQuery(
	params: Pick<URLSearchParams, "get">,
): URLSearchParams {
	const query = new URLSearchParams();
	const type = exploreTypeFromParam(params.get("type"));
	if (type !== "all") query.set("type", type);
	const q = params.get("q")?.trim();
	if (q) query.set("q", q);
	const category =
		appCategoryOf(params.get("category")) ??
		(params.get("categories") ?? "")
			.split(",")
			.map((entry) => entry.trim())
			.filter((entry) => entry.startsWith("app:"))
			.map((entry) => appCategoryOf(entry.slice("app:".length)))
			.find((entry) => entry !== undefined);
	if (category) query.set("category", category);
	const sort = LEGACY_SORTS.get(params.get("sort") ?? "");
	if (sort) query.set("sort", sort);
	return query;
}

function sortedQuery(params: URLSearchParams): string {
	const copy = new URLSearchParams(params);
	copy.sort();
	return copy.toString();
}

/**
 * Where the legacy fallback has to go before it can render: the old package list when a developer asked for
 * packages, or the same path with the legacy apps query when the URL still speaks Browse. `null` once the URL is
 * already in legacy form. Without developer mode `type=packages` reads as apps.
 */
export function legacyFallbackTarget({
	developerMode,
	pathname,
	searchParams,
}: {
	developerMode: boolean;
	pathname: string;
	searchParams: URLSearchParams;
}): string | null {
	if (developerMode && searchParams.get("type") === "packages") {
		return LEGACY_PACKAGES_HREF;
	}
	const legacy = legacyAppsQuery(searchParams);
	if (sortedQuery(legacy) === sortedQuery(searchParams)) return null;
	const query = legacy.toString();
	return query ? `${pathname}?${query}` : pathname;
}
