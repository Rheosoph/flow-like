export type QueryParamRecord = Record<string, string | string[]>;
export type PageContextMode = "none" | "path" | "query";

export interface PageContext {
	pathname: string;
	routePathname: string;
	search: string;
	hash: string;
	queryParams: QueryParamRecord;
}

export interface PageContextOptions {
	mode?: PageContextMode | string | null;
	queryParamAllowlist?: string[] | string | null;
	queryParamDenylist?: string[] | string | null;
	includeHash?: boolean;
}

function collectSearchParams(search: string | URLSearchParams) {
	return search instanceof URLSearchParams
		? search
		: new URLSearchParams(search);
}

function normalizeQueryParamList(
	value: string[] | string | null | undefined,
): Set<string> | null {
	if (!value) return null;

	const values = Array.isArray(value)
		? value
		: value
				.split(",")
				.map((part) => part.trim())
				.filter(Boolean);
	const normalized = values
		.map((part) => part.trim())
		.filter(Boolean);

	return normalized.length > 0 ? new Set(normalized) : null;
}

export function normalizePageContextMode(
	mode: PageContextOptions["mode"],
	fallback: PageContextMode = "path",
): PageContextMode {
	if (mode === "none" || mode === "path" || mode === "query") return mode;
	if (typeof mode !== "string") return fallback;

	const normalized = mode.trim().toLowerCase();
	if (
		normalized === "off" ||
		normalized === "false" ||
		normalized === "disabled"
	) {
		return "none";
	}
	if (normalized === "pathname" || normalized === "path-only") {
		return "path";
	}
	if (
		normalized === "all" ||
		normalized === "full" ||
		normalized === "queryparams" ||
		normalized === "query-params" ||
		normalized === "pathandquery" ||
		normalized === "path-and-query"
	) {
		return "query";
	}

	return fallback;
}

function filterSearchParams(
	search: string | URLSearchParams,
	options: PageContextOptions,
) {
	const params = collectSearchParams(search);
	const allowlist = normalizeQueryParamList(options.queryParamAllowlist);
	const denylist = normalizeQueryParamList(options.queryParamDenylist);
	const filtered = new URLSearchParams();

	params.forEach((value, key) => {
		if (allowlist && !allowlist.has(key)) return;
		if (denylist?.has(key)) return;
		filtered.append(key, value);
	});

	return filtered;
}

export function queryParamsToRecord(
	search: string | URLSearchParams,
): QueryParamRecord {
	const params = collectSearchParams(search);
	const result: QueryParamRecord = {};

	params.forEach((value, key) => {
		const existing = result[key];
		if (existing === undefined) {
			result[key] = value;
		} else if (Array.isArray(existing)) {
			existing.push(value);
		} else {
			result[key] = [existing, value];
		}
	});

	return result;
}

export function canonicalizeSearchParams(search: string | URLSearchParams) {
	const pairs: Array<[string, string]> = [];
	collectSearchParams(search).forEach((value, key) => {
		pairs.push([key, value]);
	});

	pairs.sort(([keyA, valueA], [keyB, valueB]) => {
		const keyCompare = keyA.localeCompare(keyB);
		if (keyCompare !== 0) return keyCompare;
		return valueA.localeCompare(valueB);
	});

	const canonical = new URLSearchParams();
	for (const [key, value] of pairs) {
		canonical.append(key, value);
	}

	return canonical.toString();
}

export function getPageContextStorageId(pathname: string, search: string) {
	const canonicalSearch = canonicalizeSearchParams(search);
	return `${pathname || "/"}${canonicalSearch ? `?${canonicalSearch}` : ""}`;
}

export function getCurrentPageContext(
	pathname?: string | null,
	options: PageContextOptions = {},
): PageContext | null {
	const mode = normalizePageContextMode(options.mode);
	if (mode === "none") return null;

	if (typeof window === "undefined") {
		return {
			pathname: pathname ?? "",
			routePathname: pathname ?? "",
			search: "",
			hash: "",
			queryParams: {},
		};
	}

	const { location } = window;
	const filteredSearch =
		mode === "query"
			? filterSearchParams(location.search, options)
			: new URLSearchParams();
	const search = filteredSearch.toString();

	return {
		pathname: location.pathname || pathname || "",
		routePathname: pathname ?? "",
		search: search ? `?${search}` : "",
		hash: options.includeHash ? location.hash : "",
		queryParams: queryParamsToRecord(filteredSearch),
	};
}
