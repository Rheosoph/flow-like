/*
 * The route rule a device applies to an `http` or `api` event (design R2 §1.2).
 * The device's listener is the authority; this copy refuses the same routes
 * before anything is uploaded.
 */

export const ROUTE_METHODS = [
	"GET",
	"POST",
	"PUT",
	"PATCH",
	"DELETE",
	"HEAD",
	"OPTIONS",
] as const;
export type RouteMethod = (typeof ROUTE_METHODS)[number];

export type EventRoute = { method: RouteMethod; path: string };

export const ROUTE_PROBLEMS = [
	"route_missing",
	"route_invalid",
	"route_reserved",
] as const;
export type RouteProblem = (typeof ROUTE_PROBLEMS)[number];

export type DeviceRouteResult =
	| {
			ok: true;
			route: EventRoute;
			/** `path` starts with `/` and `method` is set: agents before Endpoints read only this form. */
			strict: boolean;
	  }
	| {
			ok: false;
			problem: RouteProblem;
			/** `route_invalid`: the method or path as written; `route_reserved`: the path. */
			detail?: string;
	  };

export const MAX_ROUTE_PATH_BYTES = 2048;
/** The event types whose route comes from their config. */
export const ROUTE_EVENT_TYPES = ["http", "api"] as const;

const shown = (text: string) =>
	text.length > 64 ? `${text.slice(0, 63)}…` : text;

function readablePath(path: string): boolean {
	return (
		path.length <= MAX_ROUTE_PATH_BYTES &&
		/^[\x21-\x7e]+$/.test(path) &&
		!/[?#\\{}*]/.test(path) &&
		!path.split("/").some((segment) => segment === "." || segment === "..")
	);
}

/** Paths the service host serves itself (`sa/hosting.rs`); nothing is ever added to this list. */
export function reservedPath(path: string): boolean {
	return (
		path === "/services" ||
		path === "/ui" ||
		path.startsWith("/ui/") ||
		path.startsWith("/channels/")
	);
}

/** The route a device serves for this config, or why it serves none. */
export function deviceRoute(config: unknown): DeviceRouteResult {
	if (!config || typeof config !== "object" || Array.isArray(config))
		return { ok: false, problem: "route_missing" };
	const record = config as Record<string, unknown>;
	const written =
		typeof record.path === "string"
			? record.path
			: typeof record.path_suffix === "string"
				? record.path_suffix
				: undefined;
	if (written === undefined) return { ok: false, problem: "route_missing" };
	const path = written.startsWith("/") ? written : `/${written}`;
	if (!readablePath(path))
		return { ok: false, problem: "route_invalid", detail: shown(written) };
	const method = ROUTE_METHODS.find((known) =>
		record.method == null
			? known === "POST"
			: typeof record.method === "string" &&
				known === record.method.toUpperCase(),
	);
	if (method === undefined)
		return {
			ok: false,
			problem: "route_invalid",
			detail: shown(
				typeof record.method === "string"
					? record.method
					: JSON.stringify(record.method),
			),
		};
	if (reservedPath(path))
		return { ok: false, problem: "route_reserved", detail: shown(path) };
	return {
		ok: true,
		route: { method, path },
		strict:
			typeof record.path === "string" &&
			record.path.startsWith("/") &&
			typeof record.method === "string",
	};
}

/** What `routeConflicts` reads of an event. */
export interface RouteClaimEvent {
	id: string;
	event_type: string;
	default_page_id?: string | null;
	/** The event's own route (an `http` or `api` event that passed `deviceRoute`). */
	route?: EventRoute | null;
}

export interface RouteConflict {
	method: string;
	path: string;
	/** In the order the events were given. */
	eventIds: string[];
}

const PERSON_STARTED = ["quick_action", "generic_form"];

/** The routes one event occupies on its service's listener (`route_claims` in `sa/hosting.rs`). */
export function routeClaims(
	event: RouteClaimEvent,
	hasWebEndpoint: boolean,
): EventRoute[] {
	if (event.default_page_id)
		return [
			{ method: "GET", path: `/pages/${event.id}/bootstrap` },
			{ method: "POST", path: `/pages/${event.id}/invoke` },
		];
	if (event.event_type === "simple_chat")
		return [{ method: "POST", path: `/chat/${event.id}` }];
	if (PERSON_STARTED.includes(event.event_type))
		return hasWebEndpoint ? [{ method: "POST", path: `/run/${event.id}` }] : [];
	return event.route &&
		(ROUTE_EVENT_TYPES as readonly string[]).includes(event.event_type)
		? [event.route]
		: [];
}

/**
 * Routes that two events of one service claim: their own routes, chat routes,
 * Page endpoints and, in a service with a web endpoint, the run route of a form
 * or quick action. Ordered by first claim.
 */
export function routeConflicts(
	events: readonly RouteClaimEvent[],
	hasWebEndpoint: boolean,
): RouteConflict[] {
	const claims = new Map<string, RouteConflict>();
	for (const event of events)
		for (const route of routeClaims(event, hasWebEndpoint)) {
			const key = `${route.method} ${route.path}`;
			const claim = claims.get(key) ?? { ...route, eventIds: [] };
			if (!claim.eventIds.includes(event.id)) claim.eventIds.push(event.id);
			claims.set(key, claim);
		}
	return [...claims.values()].filter((claim) => claim.eventIds.length > 1);
}
