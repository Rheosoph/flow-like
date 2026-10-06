import { normalizeRoutePath, routePathsEqual } from "../route-path";
import type { IEvent } from "../schema/flow/event";
import { isUsePathname, readUseRoutePath } from "../use-route-url";

/** The static export has exactly one document; every app route renders inside it. */
export const SERVICE_BASE = "/ui/";

export type ServiceNavigation =
	| { kind: "event"; eventId: string; href: string }
	| { kind: "query"; href: string }
	| { kind: "external"; href: string }
	| { kind: "unsupported" };

const unsupported: ServiceNavigation = { kind: "unsupported" };

function servicePath(pathname: string): string {
	if (pathname === "/ui") return "/";
	return pathname.startsWith(SERVICE_BASE) ? pathname.slice(3) : pathname;
}

function serviceHref(params: URLSearchParams, hash: string): string {
	const search = params.toString();
	return `${SERVICE_BASE}${search ? `?${search}` : ""}${hash}`;
}

function routeEvent(
	route: string | null,
	events: readonly IEvent[],
): IEvent | undefined {
	if (route === null)
		return events.find((event) => event.is_default) ?? events[0];
	return (
		events.find(
			(event) => event.route && routePathsEqual(event.route, route),
		) ??
		(normalizeRoutePath(route) === "/"
			? events.find((event) => event.is_default)
			: undefined)
	);
}

/** Read the route a reloaded or shared service URL points at. */
export function initialServiceEvent(
	search: string,
	events: readonly IEvent[],
): IEvent | undefined {
	const route = new URLSearchParams(search).get("route");
	return (
		(route === null ? undefined : routeEvent(route, events)) ??
		routeEvent(null, events)
	);
}

/**
 * Map app links (`/use?id=…&route=…`, `/use/<route>`) and query updates onto the service
 * shell, so navigation selects a deployed interface instead of leaving the unlocked tab.
 */
export function resolveServiceNavigation(
	href: string,
	projectId: string,
	events: readonly IEvent[],
	current: URL,
): ServiceNavigation {
	let url: URL;
	try {
		url = new URL(href, current);
	} catch {
		return unsupported;
	}
	if (url.origin !== current.origin)
		return url.protocol === "https:" || url.protocol === "http:"
			? { kind: "external", href: url.href }
			: unsupported;
	const pathname = servicePath(url.pathname);
	const params = new URLSearchParams(url.search);
	let route: string | null;
	if (isUsePathname(pathname)) {
		const appId = params.get("id");
		if (appId !== null && appId !== projectId) return unsupported;
		try {
			route = readUseRoutePath(pathname) ?? params.get("route");
		} catch {
			return unsupported;
		}
	} else if (pathname === "/") {
		route = params.get("route");
		if (route === null) {
			// A query update keeps the interface on screen, so its URL must keep that route.
			const shown = current.searchParams.get("route");
			if (shown !== null) params.set("route", shown);
			return { kind: "query", href: serviceHref(params, url.hash) };
		}
	} else {
		return unsupported;
	}
	const event = routeEvent(route, events);
	if (!event) return unsupported;
	params.delete("id");
	if (event.route) params.set("route", normalizeRoutePath(event.route));
	else params.delete("route");
	return {
		kind: "event",
		eventId: event.id,
		href: serviceHref(params, url.hash),
	};
}

const absoluteHref = /^https?:/i;

function withQuery(href: string, queryParams: Record<string, string>): string {
	const entries = Object.entries(queryParams);
	if (entries.length === 0) return href;
	let url: URL;
	try {
		url = new URL(href, "https://service.invalid");
	} catch {
		return href;
	}
	for (const [key, value] of entries) url.searchParams.set(key, value);
	return absoluteHref.test(href)
		? url.href
		: `${url.pathname}${url.search}${url.hash}`;
}

/**
 * Chat navigation hands over a route and its query parameters separately. App links and
 * external URLs keep their target, as in the web app; bare routes become app links.
 */
export function appRouteHref(
	route: string,
	queryParams: Record<string, string> = {},
): string {
	const separator = route.indexOf("?");
	const path = separator < 0 ? route : route.slice(0, separator);
	if (isUsePathname(path) || absoluteHref.test(route))
		return withQuery(route, queryParams);
	const params = new URLSearchParams(
		separator < 0 ? "" : route.slice(separator + 1),
	);
	for (const [key, value] of Object.entries(queryParams))
		params.set(key, value);
	params.set("route", path || "/");
	return `/use?${params.toString()}`;
}

/** The shell URL that reopens an interface picked from the selector. */
export function serviceEventHref(event: IEvent): string {
	return event.route
		? serviceHref(
				new URLSearchParams({ route: normalizeRoutePath(event.route) }),
				"",
			)
		: SERVICE_BASE;
}
