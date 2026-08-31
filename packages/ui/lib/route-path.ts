/**
 * Canonical form of an app route path.
 *
 * Route paths are authored by hand in event settings, emitted by the Notify User
 * node's link builder, and compared against the `route` query param of `/use`.
 * Those three sites used to normalize differently (or not at all) while the
 * resolver compared with `===`, so a route saved as `/config/` never matched a
 * link for `/config` — and the resolver's miss is silent, rendering the default
 * route instead. One normalizer keeps writer and reader on the same string.
 *
 * Case is preserved: two paths differing only in case are two different routes,
 * and folding them would merge distinct mappings.
 */
export function normalizeRoutePath(path: unknown): string {
	const raw = String(path ?? "").trim();
	if (!raw) return "/";

	const withoutFragment = raw.split("#")[0] ?? raw;
	const withoutQuery = withoutFragment.split("?")[0] ?? withoutFragment;
	const withLeadingSlash = withoutQuery.startsWith("/")
		? withoutQuery
		: `/${withoutQuery}`;
	const withoutTrailingSlash = withLeadingSlash.replace(/\/+$/, "");

	return withoutTrailingSlash === "" ? "/" : withoutTrailingSlash;
}

/** Whether two route paths address the same route. */
export function routePathsEqual(left: unknown, right: unknown): boolean {
	return normalizeRoutePath(left) === normalizeRoutePath(right);
}
