import type { IProfile, QueryClient } from "@flow-like/flow-like-ui";
import {
	apiErrorDiagnostic,
	apiResponseError,
	redactApiPathSecrets,
	upstreamFailureInSuccess,
} from "@flow-like/flow-like-ui/lib/api-error";
import { getApiOrigin, getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import {
	BOARD_FORMAT_HEADER,
	CURRENT_BOARD_FORMAT_VERSION,
} from "@flow-like/flow-like-ui/lib/board-format";
import {
	requestTimeoutMs,
	withRequestDeadline,
} from "@flow-like/flow-like-ui/lib/request-deadline";
import type { AuthContextProps } from "react-oidc-context";

const PROTECTED_APP_ROUTE_SEGMENTS = new Set([
	"payments",
	"marketplace",
	"analytics",
	"api",
	"board",
	"comments",
	"connections",
	"data",
	"db",
	"events",
	"flowpilot-builds",
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
	return getApiOrigin();
}

export function constructApiUrl(path: string): string {
	return getApiUrl(null, path);
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
	// `apps/fork/jobs/{job_id}` only ever returns the caller's own job, so the
	// token has to be on the request rather than renewed after a 401.
	if (appOrRoute === "fork" && parts[2] === "jobs") return true;

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
		requestSilentRenew(auth, "before API request");
	}

	throw new Error(
		`Authentication token required for app request: ${redactApiPathSecrets(path)}`,
	);
}

export function requestSilentRenew(
	auth: AuthContextProps,
	reason: string,
): void {
	try {
		void Promise.resolve(auth.startSilentRenew()).catch(() => {
			console.warn(`[Auth] Silent renew failed ${reason}`);
		});
	} catch {
		console.warn(`[Auth] Silent renew failed ${reason}`);
	}
}

export async function apiFetch<T>(
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
): Promise<T> {
	const method = methodOf(options);
	ensureProtectedAppRouteAuth(path, auth, method);
	const headers: HeadersInit = {
		"Content-Type": "application/json",
		[BOARD_FORMAT_HEADER]: String(CURRENT_BOARD_FORMAT_VERSION),
	};

	if (auth?.user?.access_token) {
		headers.Authorization = `Bearer ${auth.user.access_token}`;
	}

	const url = constructApiUrl(path);
	const safePath = redactApiPathSecrets(path) ?? path;
	const { signal, ...init } = options ?? {};
	const bodyBytes = typeof init.body === "string" ? init.body.length : 0;
	// The deadline spans the body read too: a stalled body parks the caller as
	// surely as stalled headers do.
	return withRequestDeadline<T>(
		safePath,
		async (deadline) => {
			const response = await fetch(url, {
				...init,
				headers: {
					...headers,
					...init.headers,
				},
				signal: deadline.signal,
			});

			if (!response.ok) {
				if (response.status === 401 && auth) {
					requestSilentRenew(auth, "after 401");
				}
				const errorText = await response.text();
				const error = apiResponseError(response, errorText, path);
				console.error(
					`API error ${response.status} for ${safePath}:`,
					apiErrorDiagnostic(error),
				);
				throw error;
			}

			const text = await response.text();
			if (!text) return undefined as T;

			let data: unknown;
			try {
				data = JSON.parse(text);
			} catch {
				data = text;
			}
			const upstreamError = upstreamFailureInSuccess(response, data, path);
			if (upstreamError) {
				console.error(
					`API error for ${safePath}:`,
					apiErrorDiagnostic(upstreamError),
				);
				throw upstreamError;
			}
			return data as T;
		},
		{
			timeoutMs: requestTimeoutMs(cleanApiPath(path), method, bodyBytes),
			signal,
		},
	);
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
