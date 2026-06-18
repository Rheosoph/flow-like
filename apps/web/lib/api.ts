import type { IProfile } from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import type { AuthContextProps } from "react-oidc-context";

const PROTECTED_APP_ROUTE_SEGMENTS = new Set([
	"analytics",
	"api",
	"board",
	"comments",
	"data",
	"db",
	"events",
	"fork",
	"graph",
	"invoke",
	"nodes",
	"notifications",
	"packages",
	"pages",
	"publication",
	"roles",
	"routes",
	"sales",
	"settings",
	"team",
	"templates",
	"visibility",
	"widgets",
]);

function constructUrl(profile: IProfile, path: string): string {
	return getApiUrl(profile, path);
}

function cleanApiPath(path: string): string {
	return path
		.replace(/^\/+/, "")
		.replace(/^api\/v1\/+/, "")
		.split(/[?#]/, 1)[0];
}

function isProtectedAppRoute(path: string, method: string): boolean {
	const parts = cleanApiPath(path).split("/").filter(Boolean);
	if (parts[0] !== "apps" || parts.length < 2) return false;

	const appOrRoute = parts[1];
	if (appOrRoute === "search" || appOrRoute === "nodes") return false;
	if (appOrRoute === "new") return true;

	if (parts.length === 2) return method !== "GET";

	const segment = parts[2];
	if (segment === "comments") return method !== "GET";
	if (segment === "fork" && parts[3] === "preview" && method === "GET") {
		return false;
	}
	if (
		segment === "fork" &&
		parts[3] === "offline" &&
		parts[4] === "begin" &&
		method === "POST"
	) {
		return false;
	}
	if (segment === "meta") return method !== "GET";
	return PROTECTED_APP_ROUTE_SEGMENTS.has(segment);
}

function ensureProtectedAppRouteAuth(
	path: string,
	auth?: AuthContextProps,
	method = "GET",
): void {
	if (!isProtectedAppRoute(path, method)) return;
	if (auth?.user?.access_token) return;

	if (auth?.isAuthenticated) {
		try {
			auth.startSilentRenew();
		} catch (error) {
			console.warn("[Auth] Silent renew failed before API request:", error);
		}
	}

	throw new Error(`Authentication token required for app request: ${path}`);
}

export async function get<T>(
	profile: IProfile,
	path: string,
	auth?: AuthContextProps,
): Promise<T | undefined> {
	ensureProtectedAppRouteAuth(path, auth, "GET");
	const authHeader: Record<string, string> = auth?.user?.access_token
		? { Authorization: `Bearer ${auth.user.access_token}` }
		: {};

	const url = constructUrl(profile, path);
	const response = await fetch(url, {
		method: "GET",
		headers: {
			"Content-Type": "application/json",
			...authHeader,
		},
	});

	if (!response.ok) {
		console.error(`HTTP error: ${response.status}`, await response.text());
		return undefined;
	}

	return (await response.json()) as T;
}

export async function post<T>(
	profile: IProfile,
	path: string,
	body?: any,
	auth?: AuthContextProps,
): Promise<T | undefined> {
	ensureProtectedAppRouteAuth(path, auth, "POST");
	const authHeader: Record<string, string> = auth?.user?.access_token
		? { Authorization: `Bearer ${auth.user.access_token}` }
		: {};

	const url = constructUrl(profile, path);
	const response = await fetch(url, {
		method: "POST",
		headers: {
			"Content-Type": "application/json",
			...authHeader,
		},
		body: body ? JSON.stringify(body) : undefined,
	});

	if (!response.ok) {
		console.error(`HTTP error: ${response.status}`, await response.text());
		return undefined;
	}

	return (await response.json()) as T;
}

export async function put<T>(
	profile: IProfile,
	path: string,
	body?: any,
	auth?: AuthContextProps,
): Promise<T | undefined> {
	ensureProtectedAppRouteAuth(path, auth, "PUT");
	const authHeader: Record<string, string> = auth?.user?.access_token
		? { Authorization: `Bearer ${auth.user.access_token}` }
		: {};

	const url = constructUrl(profile, path);
	const response = await fetch(url, {
		method: "PUT",
		headers: {
			"Content-Type": "application/json",
			...authHeader,
		},
		body: body ? JSON.stringify(body) : undefined,
	});

	if (!response.ok) {
		console.error(`HTTP error: ${response.status}`, await response.text());
		return undefined;
	}

	return (await response.json()) as T;
}

export async function del<T>(
	profile: IProfile,
	path: string,
	auth?: AuthContextProps,
): Promise<T | undefined> {
	ensureProtectedAppRouteAuth(path, auth, "DELETE");
	const authHeader: Record<string, string> = auth?.user?.access_token
		? { Authorization: `Bearer ${auth.user.access_token}` }
		: {};

	const url = constructUrl(profile, path);
	const response = await fetch(url, {
		method: "DELETE",
		headers: {
			"Content-Type": "application/json",
			...authHeader,
		},
	});

	if (!response.ok) {
		console.error(`HTTP error: ${response.status}`, await response.text());
		return undefined;
	}

	return (await response.json()) as T;
}

export async function fetcher<T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
): Promise<T> {
	ensureProtectedAppRouteAuth(
		path,
		auth,
		(options?.method ?? "GET").toUpperCase(),
	);
	const headers: HeadersInit = {};
	if (auth?.user?.access_token) {
		headers["Authorization"] = `Bearer ${auth?.user?.access_token}`;
	}

	// Check network status before attempting request
	if (typeof navigator !== "undefined" && !navigator.onLine) {
		console.warn(`Network offline - request will use cache: ${path}`);
		throw new Error(`Network unavailable: ${path}`);
	}

	const url = constructUrl(profile, path);
	try {
		const response = await fetch(url, {
			...options,
			headers: {
				"Content-Type": "application/json",
				...options?.headers,
				...headers,
			},
			keepalive: true,
		});

		if (!response.ok) {
			if (response.status === 401 && auth) {
				auth?.startSilentRenew();
			}
			console.error(`Error fetching ${path}:`, response);
			console.error(await response.text());
			throw new Error(`HTTP error! status: ${response.status}`);
		}

		return await response.json();
	} catch (error) {
		console.error(`Error fetching ${path}:`, error);
		throw error;
	}
}
