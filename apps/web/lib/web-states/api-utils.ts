import type { QueryClient } from "@tanstack/react-query";
import type { IProfile } from "@flow-like/flow-like-ui";
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

export interface WebBackendRef {
	profile?: IProfile;
	auth?: AuthContextProps;
	queryClient?: QueryClient;
}

export function getApiBaseUrl(): string {
	return process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";
}

export function constructApiUrl(path: string): string {
	const baseUrl = getApiBaseUrl();
	const cleanBase = baseUrl.endsWith("/") ? baseUrl.slice(0, -1) : baseUrl;
	const cleanPath = path.replace(/^\/+/, "");
	return `${cleanBase}/api/v1/${cleanPath}`;
}

function jsonStringify(value: unknown): string {
	return JSON.stringify(value, (_key, v) =>
		typeof v === "bigint" ? Number(v) : v,
	);
}

function cleanApiPath(path: string): string {
	return path
		.replace(/^\/+/, "")
		.replace(/^api\/v1\/+/, "")
		.split(/[?#]/, 1)[0];
}

function methodOf(options?: RequestInit): string {
	return (options?.method ?? "GET").toUpperCase();
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

export function ensureProtectedAppRouteAuth(
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

function apiErrorMessage(status: number, body: string): string {
	if (body) {
		try {
			const parsed = JSON.parse(body);
			const message = parsed?.error?.message ?? parsed?.message ?? parsed?.error;
			if (typeof message === "string" && message.trim()) {
				return message;
			}
		} catch {
			const trimmed = body.trim();
			if (trimmed) return trimmed;
		}
	}
	return `API error: ${status}`;
}

export async function apiFetch<T>(
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
): Promise<T> {
	ensureProtectedAppRouteAuth(path, auth, methodOf(options));
	const headers: HeadersInit = {
		"Content-Type": "application/json",
	};

	if (auth?.user?.access_token) {
		headers["Authorization"] = `Bearer ${auth.user.access_token}`;
	}

	const url = constructApiUrl(path);
	const response = await fetch(url, {
		...options,
		headers: {
			...headers,
			...options?.headers,
		},
	});

	if (!response.ok) {
		if (response.status === 401 && auth?.isAuthenticated) {
			auth.startSilentRenew();
		}
		const errorText = await response.text();
		console.error(`API error ${response.status} for ${path}:`, errorText);
		throw new Error(apiErrorMessage(response.status, errorText));
	}

	const text = await response.text();
	if (!text) return undefined as T;

	try {
		return JSON.parse(text) as T;
	} catch {
		return text as T;
	}
}

export async function apiGet<T>(
	path: string,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(path, { method: "GET" }, auth);
}

export async function apiPost<T>(
	path: string,
	body?: unknown,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "POST",
			body: body ? jsonStringify(body) : undefined,
		},
		auth,
	);
}

export async function apiPut<T>(
	path: string,
	body?: unknown,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "PUT",
			body: body ? jsonStringify(body) : undefined,
		},
		auth,
	);
}

export async function apiPatch<T>(
	path: string,
	body?: unknown,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "PATCH",
			body: body ? jsonStringify(body) : undefined,
		},
		auth,
	);
}

export async function apiDelete<T>(
	path: string,
	auth?: AuthContextProps,
	body?: unknown,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "DELETE",
			...(body
				? {
						headers: { "Content-Type": "application/json" },
						body: JSON.stringify(body),
					}
				: {}),
		},
		auth,
	);
}
