import { normalizeRoutePath } from "./route-path";

/** App-owned query data is nested so names such as `id` cannot select another app. */
export const APP_QUERY_PARAM = "appQuery";

export interface AppQueryParam {
	name: string;
	value: string;
}

export interface AppRouteTarget {
	path?: string;
	queryParams: AppQueryParam[];
}

const controls = /\p{Cc}/u;
const bytes = (value: string) => new TextEncoder().encode(value).length;

function assertInternalPath(path: string) {
	if (
		/^[a-z][a-z0-9+.-]*:/i.test(path) ||
		path.startsWith("//") ||
		path.includes("\\") ||
		path.split("/").some((segment) => segment === "." || segment === "..")
	)
		throw new Error("Choose an internal path within this app.");
}

/** Validate native input without URL normalization silently removing traversal segments. */
export function parseAppRouteTarget(
	path: unknown = undefined,
	queryParams: unknown = undefined,
): AppRouteTarget {
	if (path !== undefined && typeof path !== "string")
		throw new Error("The app path must be text.");
	const raw = (path ?? "") as string;
	if (bytes(raw) > 4096 || controls.test(raw) || raw.includes("#"))
		throw new Error(
			"The app path is too long or contains an invalid character.",
		);
	const separator = raw.indexOf("?");
	const pathPart = (separator < 0 ? raw : raw.slice(0, separator)).trim();
	assertInternalPath(pathPart);
	try {
		const decoded = decodeURIComponent(pathPart);
		assertInternalPath(decoded);
		if (controls.test(decoded))
			throw new Error("The app path contains an invalid character.");
	} catch (error) {
		// A literal percent is a valid authored route character. Decode valid escapes only
		// for the safety check; the runtime matches the authored path without decoding it.
		if (!(error instanceof URIError)) throw error;
		const decoded = pathPart.replace(/%([0-9a-f]{2})/gi, (_, hex) =>
			String.fromCharCode(Number.parseInt(hex, 16)),
		);
		assertInternalPath(decoded);
		if (controls.test(decoded))
			throw new Error("The app path contains an invalid character.");
	}
	if (queryParams !== undefined && !Array.isArray(queryParams))
		throw new Error("Query parameters must be name and value pairs.");
	const entries: AppQueryParam[] = Array.from(
		new URLSearchParams(separator < 0 ? "" : raw.slice(separator + 1)),
		([name, value]) => ({ name, value }),
	);
	entries.push(...((queryParams ?? []) as AppQueryParam[]));
	if (entries.length > 32) throw new Error("Use at most 32 query parameters.");
	let total = bytes(raw);
	for (const pair of entries) {
		if (
			!pair ||
			typeof pair.name !== "string" ||
			typeof pair.value !== "string" ||
			!pair.name ||
			bytes(pair.name) > 256 ||
			bytes(pair.value) > 4096 ||
			controls.test(pair.name) ||
			controls.test(pair.value)
		)
			throw new Error("A query parameter has an invalid name or value.");
	}
	for (const pair of (queryParams ?? []) as AppQueryParam[])
		total += bytes(pair.name) + bytes(pair.value);
	if (total > 8192)
		throw new Error("The app path and query parameters are too long.");
	return {
		path: pathPart ? normalizeRoutePath(pathPart) : undefined,
		queryParams: entries.map(({ name, value }) => ({ name, value })),
	};
}

export function appRouteUrl(appId: string, target: AppRouteTarget): string {
	const outer = new URLSearchParams({ id: appId });
	if (target.path) outer.set("route", target.path);
	if (target.path || target.queryParams.length) {
		const inner = new URLSearchParams();
		for (const { name, value } of target.queryParams) inner.append(name, value);
		outer.set(APP_QUERY_PARAM, inner.toString());
	}
	return `/use?${outer.toString()}`;
}

export function readAppQuery(search: string): URLSearchParams {
	const outer = new URLSearchParams(search);
	return outer.has(APP_QUERY_PARAM)
		? new URLSearchParams(outer.get(APP_QUERY_PARAM) ?? "")
		: outer;
}

/** Preserve the legacy scalar lookup while exposing every repeated value to the Event. */
export function appQueryContext(search: string): {
	_query_params: Record<string, string>;
	_query_params_format?: "app";
	_query_param_values?: Record<string, string[]>;
} {
	const params = readAppQuery(search);
	const result = { _query_params: Object.fromEntries(params) };
	if (!new URLSearchParams(search).has(APP_QUERY_PARAM)) return result;
	const values: Record<string, string[]> = Object.create(null);
	for (const [name, value] of params) {
		values[name] ??= [];
		values[name].push(value);
	}
	return {
		...result,
		_query_params_format: "app",
		_query_param_values: values,
	};
}

/** Query updates from a page stay within its existing app-owned query envelope. */
export function setAppQueryParam(url: URL, key: string, value?: string): void {
	const nested = url.searchParams.has(APP_QUERY_PARAM);
	const params = nested ? readAppQuery(url.search) : url.searchParams;
	if (value === undefined || value === "") params.delete(key);
	else params.set(key, value);
	if (nested) url.searchParams.set(APP_QUERY_PARAM, params.toString());
}
