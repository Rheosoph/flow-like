import { normalizeRoutePath } from "@flow-like/flow-like-ui/lib/route-path";
import { readUseRoutePath } from "@flow-like/flow-like-ui/lib/use-route-url";

export interface HostedTarget {
	app: string;
	route: string;
	variant?: string;
}

const APP_ID = /^[a-zA-Z0-9_-]{1,128}$/;
/** A published route is normalized, so it can never carry these. */
const UNSAFE_ROUTE = /[\\?#]|\p{Cc}/u;
const RESERVED_NAVIGATION_PARAMS = [
	"id",
	"app",
	"event",
	"eventId",
	"route",
	"sessionId",
] as const;
const INVALID_NAVIGATION =
	"This navigation does not point to a published app route.";

export function isHostedFrontendPath(pathname: string | null): boolean {
	return pathname !== null && /^\/a(?:\/|$)/.test(pathname);
}

/** `/a?app=<app>&route=/x` works on static hosts without a rewrite rule. */
export function isHostedQueryPath(pathname: string): boolean {
	return /^\/a\/?$/.test(pathname);
}

function hostedLocation(url: URL): { app: string; route: string } | null {
	if (isHostedQueryPath(url.pathname))
		return {
			app: url.searchParams.get("app") ?? "",
			route: url.searchParams.get("route") ?? "/",
		};
	if (!url.pathname.startsWith("/a/")) return null;
	try {
		const [app, ...route] = url.pathname
			.slice("/a/".length)
			.split("/")
			.map(decodeURIComponent);
		if (route.some((segment) => segment.includes("/"))) return null;
		return { app, route: `/${route.join("/")}` };
	} catch {
		return null;
	}
}

export function parseHostedTarget(url: URL): HostedTarget | null {
	const location = hostedLocation(url);
	if (
		!location ||
		!APP_ID.test(location.app) ||
		UNSAFE_ROUTE.test(location.route)
	)
		return null;
	const variant = url.searchParams.get("__variant");
	return {
		app: location.app,
		route: normalizeRoutePath(location.route),
		...(variant ? { variant } : {}),
	};
}

/** Auth can only return to a hosted surface on this same origin. */
export function hostedReturnPath(value: unknown): string | null {
	if (
		typeof value !== "string" ||
		!value.startsWith("/") ||
		value.includes("\\") ||
		Array.from(value).some(
			(character) =>
				character.charCodeAt(0) <= 32 || character.charCodeAt(0) === 127,
		)
	)
		return null;
	const origin = "https://hosted.invalid";
	try {
		const url = new URL(value, origin);
		if (url.origin !== origin || !parseHostedTarget(url)) return null;
		return url.pathname + url.search + url.hash;
	} catch {
		return null;
	}
}

export function hostedApiPath(target: HostedTarget): string {
	return `frontend/a/${encodeURIComponent(target.app)}`;
}

function hostedPathname(app: string, route: string): string {
	const path =
		route === "/" ? "" : route.split("/").map(encodeURIComponent).join("/");
	return `/a/${encodeURIComponent(app)}${path}`;
}

/** App links arrive as a bare route, `/use?route=/x` or `/use/x`. */
function navigationRoute(url: URL): string {
	let route: string;
	try {
		route =
			url.pathname === "/use"
				? (url.searchParams.get("route") ?? "/")
				: (readUseRoutePath(url.pathname) ?? decodeURIComponent(url.pathname));
	} catch {
		throw new Error(INVALID_NAVIGATION);
	}
	if (UNSAFE_ROUTE.test(route)) throw new Error(INVALID_NAVIGATION);
	return normalizeRoutePath(route);
}

export function hostedNavigationPath(
	app: string,
	route: string,
	routes: readonly { path: string }[],
	queryParams: Record<string, string> = {},
	queryRouting = false,
): string {
	const origin = "https://hosted.invalid";
	const url = new URL(route, origin);
	if (url.origin !== origin || !/^https?:$/.test(url.protocol))
		throw new Error(INVALID_NAVIGATION);
	const path = navigationRoute(url);
	if (!routes.some((candidate) => normalizeRoutePath(candidate.path) === path))
		throw new Error(`The page ${path} has not been published on this link.`);
	const params = new URLSearchParams(url.search);
	for (const reserved of RESERVED_NAVIGATION_PARAMS) params.delete(reserved);
	for (const [key, value] of Object.entries(queryParams))
		params.set(key, value);
	if (queryRouting) {
		const query = new URLSearchParams({ app, route: path });
		for (const [key, value] of params)
			if (!query.has(key)) query.append(key, value);
		return `/a?${query}${url.hash}`;
	}
	return (
		hostedPathname(app, path) + (params.size ? `?${params}` : "") + url.hash
	);
}

let sessionId: string | undefined;
const HOSTED_SESSION_KEY = "flow-like-hosted-session";
const VALID_HOSTED_SESSION = /^[a-zA-Z0-9_-]{16,128}$/;

export function subscribeHostedSession(listener: () => void): () => void {
	const onStorage = (event: StorageEvent) => {
		if (event.key === null || event.key === HOSTED_SESSION_KEY) listener();
	};
	window.addEventListener("storage", onStorage);
	return () => window.removeEventListener("storage", onStorage);
}

/** Anonymous visits share one identity across this browser profile's tabs. */
export function hostedSessionId(): string {
	try {
		const stored = localStorage.getItem(HOSTED_SESSION_KEY);
		if (stored && VALID_HOSTED_SESSION.test(stored)) {
			sessionId = stored;
			return stored;
		}
	} catch {}
	if (!sessionId) {
		try {
			const legacy = sessionStorage.getItem(HOSTED_SESSION_KEY);
			if (legacy && VALID_HOSTED_SESSION.test(legacy)) sessionId = legacy;
		} catch {}
		sessionId ??= crypto.randomUUID();
	}
	try {
		localStorage.setItem(HOSTED_SESSION_KEY, sessionId);
		const shared = localStorage.getItem(HOSTED_SESSION_KEY);
		if (shared && VALID_HOSTED_SESSION.test(shared)) sessionId = shared;
	} catch {}
	try {
		sessionStorage.setItem(HOSTED_SESSION_KEY, sessionId);
	} catch {}
	return sessionId;
}
