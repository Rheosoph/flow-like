import type { IProfile } from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { type EventSourceMessage, createEventSource } from "eventsource-client";
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

function methodOf(options?: RequestInit, fallback = "GET"): string {
	return (options?.method ?? fallback).toUpperCase();
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
	auth?: AuthContextProps | null,
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

type SSEMessage = {
	event?: string;
	data: string;
	id?: string;
	raw: string;
};

function tryParseJSON<T>(text: string): T | null {
	try {
		return JSON.parse(text) as T;
	} catch {
		return null;
	}
}

/**
 * Parse SSE events from a text buffer.
 * Returns parsed events and remaining incomplete buffer.
 */
function parseSSEBuffer(buffer: string): {
	events: SSEMessage[];
	remaining: string;
} {
	const events: SSEMessage[] = [];
	const parts = buffer.split("\n\n");

	// Last part might be incomplete, keep it as remaining
	const remaining = parts.pop() ?? "";

	for (const part of parts) {
		if (!part.trim()) continue;

		let event: string | undefined;
		let data = "";
		let id: string | undefined;

		for (const line of part.split("\n")) {
			if (line.startsWith("event:")) {
				event = line.slice(6).trim();
			} else if (line.startsWith("data:")) {
				data = line.slice(5).trim();
			} else if (line.startsWith("id:")) {
				id = line.slice(3).trim();
			} else if (line.startsWith(":")) {
				// Comment/keep-alive, ignore
				continue;
			}
		}

		if (data) {
			events.push({ event, data, id, raw: part });
		}
	}

	return { events, remaining };
}

/**
 * Stream fetcher using raw fetch for POST requests (more reliable with Tauri)
 * and eventsource-client for GET requests.
 */
export async function streamFetcher<T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
	onMessage?: (data: T) => void,
): Promise<void> {
	const method = methodOf(options);
	ensureProtectedAppRouteAuth(path, auth, method);
	const authHeader: Record<string, string> = auth?.user?.access_token
		? { Authorization: `Bearer ${auth.user.access_token}` }
		: {};
	const url = constructUrl(profile, path);

	console.log("[SSE Debug] Starting stream to:", url);
	console.log("[SSE Debug] Method:", method);
	console.log("[SSE Debug] Has body:", !!options?.body);
	console.log("[SSE Debug] Has auth token:", !!authHeader.Authorization);

	// For POST/PUT requests, use raw fetch streaming (more reliable with Tauri)
	if (method === "POST" || method === "PUT") {
		await streamFetcherRaw<T>(url, options, authHeader, onMessage);
		return;
	}

	// For GET requests, use eventsource-client
	await streamFetcherEventSource<T>(url, options, authHeader, onMessage);
}

/**
 * Raw fetch streaming implementation for POST/PUT requests
 */
async function streamFetcherRaw<T>(
	url: string,
	options: RequestInit | undefined,
	authHeader: Record<string, string>,
	onMessage?: (data: T) => void,
): Promise<void> {
	const abortController = new AbortController();
	const response = await tauriFetch(url, {
		method: options?.method ?? "POST",
		headers: {
			Accept: "text/event-stream",
			"Content-Type": "application/json",
			...((options?.headers as Record<string, string>) ?? {}),
			...authHeader,
		},
		body: options?.body,
		signal: abortController.signal,
	});

	if (!response.ok) {
		throw new Error(`HTTP error: ${response.status} ${response.statusText}`);
	}

	if (!response.body) {
		throw new Error("Response body is null - streaming not supported");
	}

	console.log("[SSE Debug] Connected to SSE stream (raw fetch):", url);

	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = "";

	try {
		while (true) {
			const { done, value } = await reader.read();

			if (done) {
				console.log("[SSE Debug] Stream ended (done=true)");
				// Process any remaining buffer
				if (buffer.trim()) {
					const { events } = parseSSEBuffer(buffer + "\n\n");
					for (const event of events) {
						processSSEEvent(event, onMessage);
					}
				}
				break;
			}

			buffer += decoder.decode(value, { stream: true });
			const { events, remaining } = parseSSEBuffer(buffer);
			buffer = remaining;

			for (const event of events) {
				const result = processSSEEvent(event, onMessage);
				if (result === "completed" || result === "error") {
					console.log("[SSE Debug] Received terminal event:", result);
					// Use AbortController to cleanly terminate the stream
					abortController.abort();
					return;
				}
			}
		}
	} finally {
		try {
			reader.releaseLock();
		} catch {
			// Ignore errors when releasing lock - stream may already be closed
		}
	}
}

/**
 * Process a single SSE event, returns the event type for terminal detection
 */
function processSSEEvent<T>(
	event: SSEMessage,
	onMessage?: (data: T) => void,
): string {
	const evt = event.event ?? "message";
	console.log(
		"[SSE Debug] Received event:",
		evt,
		event.data?.substring(0, 200),
	);

	const parsedData = tryParseJSON<T>(event.data);
	if (parsedData && onMessage) {
		onMessage(parsedData);
	} else if (event.data && !event.data.startsWith("keep-alive")) {
		console.warn("[SSE Debug] Non-JSON data:", event.data);
	}

	// Check SSE event name and JSON data's event_type field for terminal events
	// All events from executor are InterComEvent with event_type field
	const data = parsedData as Record<string, unknown> | null;
	const eventType = data?.event_type ?? data?.type;
	if (evt === "done" || evt === "completed" || eventType === "completed") {
		return "completed";
	}
	if (evt === "error" || eventType === "error") {
		return "error";
	}
	return evt;
}

/**
 * EventSource-based streaming for GET requests
 */
async function streamFetcherEventSource<T>(
	url: string,
	options: RequestInit | undefined,
	authHeader: Record<string, string>,
	onMessage?: (data: T) => void,
): Promise<void> {
	let finished = false;

	await new Promise<void>((resolve, reject) => {
		let esRef: ReturnType<typeof createEventSource> | null = null;

		const closeAndResolve = () => {
			if (!finished) {
				finished = true;
				try {
					esRef?.close();
				} catch {}
				resolve();
			}
		};

		const closeAndReject = (error: Error) => {
			if (!finished) {
				finished = true;
				try {
					esRef?.close();
				} catch {}
				reject(error);
			}
		};

		esRef = createEventSource({
			url: url,
			fetch: tauriFetch,
			// @ts-ignore
			headers: {
				Accept: "text/event-stream",
				...(options?.body ? { "Content-Type": "application/json" } : {}),
				...(options?.headers ?? {}),
				...(authHeader.Authorization
					? { Authorization: authHeader.Authorization }
					: {}),
			},
			method: options?.method ?? "GET",
			body: options?.body ? options.body : undefined,
			signal: options?.signal,
			onMessage: (message: EventSourceMessage) => {
				console.log(
					"[SSE Debug] Received message:",
					message.event,
					message.data?.substring(0, 200),
				);
				const evt = message?.event ?? "message";
				const parsedData = tryParseJSON<T>(message.data);
				if (parsedData && onMessage) {
					onMessage(parsedData);
				} else {
					console.warn("Received non-JSON data:", message.data);
				}

				if (evt === "done" || evt === "completed") {
					closeAndResolve();
				}
				if (evt === "error") {
					closeAndReject(new Error("SSE stream error"));
				}
			},
			onConnect: () => {
				console.log("[SSE Debug] Connected to SSE stream:", url);
			},
			onScheduleReconnect: (info) => {
				console.log(
					"[SSE Debug] Preventing reconnection attempt (delay would be:",
					info.delay,
					"ms)",
				);
				closeAndResolve();
			},
			onDisconnect: () => {
				console.log("[SSE Debug] Disconnected from SSE stream:", url);
				closeAndResolve();
			},
			onError: (error: unknown) => {
				console.error("[SSE Debug] Stream error:", error);
				closeAndReject(
					error instanceof Error ? error : new Error(String(error)),
				);
			},
		});
	});
}

// --- Dev request stats: count HTTP-plugin calls per normalized endpoint so we can
// see which backend endpoint dominates the IPC traffic for online apps.
// In the app console: `__apiStats()` for a ranked list, `__apiStatsReset()` to clear.
const __apiCallStats = new Map<string, number>();
function normalizeApiPath(method: string, path: string): string {
	const base = path
		.split("?")[0]
		.replace(/[0-9a-f-]{16,}/gi, ":id")
		.replace(/\/\d+(?=\/|$)/g, "/:n");
	return `${method} ${base}`;
}
if (typeof window !== "undefined") {
	(window as unknown as Record<string, unknown>).__apiStats = () =>
		[...__apiCallStats.entries()]
			.sort((a, b) => b[1] - a[1])
			.map(([k, v]) => `${v}\t${k}`)
			.join("\n");
	(window as unknown as Record<string, unknown>).__apiStatsReset = () =>
		__apiCallStats.clear();
}

export async function fetcher<T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
): Promise<T> {
	ensureProtectedAppRouteAuth(path, auth, methodOf(options));
	const __statKey = normalizeApiPath(methodOf(options) ?? "GET", path);
	__apiCallStats.set(__statKey, (__apiCallStats.get(__statKey) ?? 0) + 1);
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
	console.log("[API DEBUG] Fetching URL:", url);
	try {
		const response = await tauriFetch(url, {
			...options,
			headers: {
				"Content-Type": "application/json",
				...options?.headers,
				...headers,
			},
			keepalive: true,
			priority: "high",
		});

		console.log("[API DEBUG] Response received:", response);

		if (!response.ok) {
			console.warn(`Error fetching ${path}:`, response);
			if (response.status === 401 && auth) {
				auth?.startSilentRenew();
			}
			console.error(`Error fetching ${path}:`, response);
			const errorText = await response.text();
			console.error(errorText);
			throw new Error(apiErrorMessage(response, errorText));
		}

		const text = await response.text();
		if (!text) return undefined as T;
		const json = tryParseJSON<T>(text);
		if (json === null) return text as T;
		console.groupCollapsed(`API Request: ${path}`);
		console.dir(json, { depth: null });
		console.groupEnd();
		return json;
	} catch (error) {
		console.groupCollapsed(`API Request: ${path}`);
		console.error(`Error fetching ${path}:`, error);
		console.groupEnd();

		// Better error messages for common network issues
		if (error instanceof Error) {
			// Network errors on mobile/desktop
			if (
				error.message.includes("Failed to fetch") ||
				error.message.includes("NetworkError") ||
				error.message.includes("Network request failed") ||
				error.message.includes("fetch failed")
			) {
				throw new Error(`Network unavailable: ${path}`);
			}
			throw error;
		}

		throw new Error(`Error fetching data: ${error}`);
	}
}

function apiErrorMessage(response: Response, body: string): string {
	if (body) {
		try {
			const parsed = JSON.parse(body);
			const message =
				parsed?.error?.message ?? parsed?.message ?? parsed?.error;
			if (typeof message === "string" && message.trim()) {
				return message;
			}
		} catch {
			const trimmed = body.trim();
			if (trimmed) return trimmed;
		}
	}

	return response.statusText || `HTTP error: ${response.status}`;
}

export async function post<T>(
	profile: IProfile,
	path: string,
	data?: any,
	auth?: AuthContextProps,
): Promise<T> {
	return fetcher<T>(
		profile,
		path,
		{
			method: "POST",
			body: data ? JSON.stringify(data) : undefined,
		},
		auth,
	);
}

export async function get<T>(
	profile: IProfile,
	path: string,
	auth?: AuthContextProps,
): Promise<T> {
	return fetcher<T>(
		profile,
		path,
		{
			method: "GET",
		},
		auth,
	);
}

export async function put<T>(
	profile: IProfile,
	path: string,
	data?: any,
	auth?: AuthContextProps,
): Promise<T> {
	return fetcher<T>(
		profile,
		path,
		{
			method: "PUT",
			body: data ? JSON.stringify(data) : undefined,
		},
		auth,
	);
}

export async function del<T>(
	profile: IProfile,
	path: string,
	data?: any,
	auth?: AuthContextProps,
): Promise<T> {
	return fetcher<T>(
		profile,
		path,
		{
			method: "DELETE",
			body: data ? JSON.stringify(data) : undefined,
		},
		auth,
	);
}

export async function patch<T>(
	profile: IProfile,
	path: string,
	data?: any,
	auth?: AuthContextProps,
): Promise<T> {
	return fetcher<T>(
		profile,
		path,
		{
			method: "PATCH",
			body: data ? JSON.stringify(data) : undefined,
		},
		auth,
	);
}
