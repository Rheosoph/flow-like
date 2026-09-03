import type { IProfile } from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { type EventSourceMessage, createEventSource } from "eventsource-client";
import type { AuthContextProps } from "react-oidc-context";
import { ApiResponseError, apiResponseError } from "./api-error";
import {
	DEFAULT_CONNECT_TIMEOUT_MS,
	STREAM_HEADER_TIMEOUT_MS,
	withRequestDeadline,
} from "./request-deadline";

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

export function requestSilentRenew(
	auth: AuthContextProps,
	reason: string,
): void {
	try {
		void Promise.resolve(auth.startSilentRenew()).catch((error) => {
			console.warn(`[Auth] Silent renew failed ${reason}:`, error);
		});
	} catch (error) {
		console.warn(`[Auth] Silent renew failed ${reason}:`, error);
	}
}

export function ensureProtectedAppRouteAuth(
	path: string,
	auth?: AuthContextProps | null,
	method = "GET",
): void {
	if (!isProtectedAppRoute(path, method)) return;
	if (auth?.user?.access_token) return;

	if (auth?.isAuthenticated) {
		requestSilentRenew(auth, "before API request");
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
		const dataLines: string[] = [];
		let id: string | undefined;

		for (const line of part.split("\n")) {
			if (line.startsWith("event:")) {
				event = line.slice(6).trim();
			} else if (line.startsWith("data:")) {
				// A payload containing newlines arrives as one data: line per
				// newline — accumulate, never overwrite. Only the optional
				// single leading space is stripped; trimming would destroy
				// whitespace-only tokens.
				const value = line.slice(5);
				dataLines.push(value.startsWith(" ") ? value.slice(1) : value);
			} else if (line.startsWith("id:")) {
				id = line.slice(3).trim();
			} else if (line.startsWith(":")) {
				// Comment/keep-alive, ignore
				continue;
			}
		}

		if (dataLines.length > 0) {
			events.push({ event, data: dataLines.join("\n"), id, raw: part });
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
	// Bounded until headers arrive, then released — the reader below owns
	// `abortController` and terminates the long-lived stream through it.
	const response = await withRequestDeadline(
		url,
		async ({ signal, release }) => {
			const res = await tauriFetch(url, {
				method: options?.method ?? "POST",
				headers: {
					Accept: "text/event-stream",
					"Content-Type": "application/json",
					...((options?.headers as Record<string, string>) ?? {}),
					...authHeader,
				},
				body: options?.body,
				connectTimeout: DEFAULT_CONNECT_TIMEOUT_MS,
				signal,
			});
			release();
			return res;
		},
		{
			timeoutMs: STREAM_HEADER_TIMEOUT_MS,
			signal: options?.signal,
			controller: abortController,
		},
	);

	if (!response.ok) {
		const errorText = await response.text();
		throw apiResponseError(response, errorText, url);
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

			// Deliver the whole decoded batch before acting on a terminal
			// event — the executor coalesces trailing chat_out/usage events
			// into the same network chunk as `completed`.
			let terminal = false;
			for (const event of events) {
				const result = processSSEEvent(event, onMessage);
				if (result === "completed" || result === "error") {
					console.log("[SSE Debug] Received terminal event:", result);
					terminal = true;
				}
			}
			if (terminal) {
				// Use AbortController to cleanly terminate the stream
				abortController.abort();
				return;
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

// --- Dev-only request stats: count HTTP-plugin calls per normalized endpoint so
// we can see which backend endpoint dominates IPC traffic for online apps. Gated
// to development so no debug globals/overhead ship in production builds.
// In the app console: `__apiStats()` for a ranked list, `__apiStatsReset()` to clear.
const API_STATS_ENABLED = process.env.NODE_ENV !== "production";
const __apiCallStats = new Map<string, number>();
function normalizeApiPath(method: string, path: string): string {
	const base = path
		.split("?")[0]
		.replace(/[0-9a-f-]{16,}/gi, ":id")
		.replace(/\/\d+(?=\/|$)/g, "/:n");
	return `${method} ${base}`;
}
if (API_STATS_ENABLED && typeof window !== "undefined") {
	(window as unknown as Record<string, unknown>).__apiStats = () =>
		[...__apiCallStats.entries()]
			.sort((a, b) => b[1] - a[1])
			.map(([k, v]) => `${v}\t${k}`)
			.join("\n");
	(window as unknown as Record<string, unknown>).__apiStatsReset = () =>
		__apiCallStats.clear();
}

/**
 * A conditional read: `notModified` says the server confirmed the caller's cached copy is
 * current and deliberately sent no body, so `data` is absent without that being a failure.
 */
export interface IConditionalResponse<T> {
	readonly notModified: boolean;
	readonly data?: T;
	readonly etag?: string;
}

/**
 * The desktop talks to the API through the Tauri HTTP plugin, which has no HTTP cache of its
 * own — a revalidation only becomes a 304 if the request carries the tag explicitly. Payloads
 * that a device already stores locally (pages above all) use this to confirm freshness without
 * re-transferring the whole document.
 */
export async function fetcherConditional<T>(
	profile: IProfile,
	path: string,
	options: RequestInit | undefined,
	auth: AuthContextProps | undefined,
	etag?: string,
): Promise<IConditionalResponse<T>> {
	return requestJson<T>(profile, path, options, auth, etag);
}

export async function fetcher<T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
): Promise<T> {
	const { data } = await requestJson<T>(profile, path, options, auth);
	return data as T;
}

async function requestJson<T>(
	profile: IProfile,
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
	ifNoneMatch?: string,
): Promise<IConditionalResponse<T>> {
	ensureProtectedAppRouteAuth(path, auth, methodOf(options));
	if (API_STATS_ENABLED) {
		const statKey = normalizeApiPath(methodOf(options), path);
		__apiCallStats.set(statKey, (__apiCallStats.get(statKey) ?? 0) + 1);
	}
	const headers: HeadersInit = {};
	if (auth?.user?.access_token) {
		headers["Authorization"] = `Bearer ${auth?.user?.access_token}`;
	}
	if (ifNoneMatch) {
		headers["If-None-Match"] = ifNoneMatch;
	}

	// Check network status before attempting request
	if (typeof navigator !== "undefined" && !navigator.onLine) {
		console.warn(`Network offline - request will use cache: ${path}`);
		throw new Error(`Network unavailable: ${path}`);
	}

	const url = constructUrl(profile, path);
	if (API_STATS_ENABLED) console.log("[API DEBUG] Fetching URL:", url);
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

		if (API_STATS_ENABLED) {
			console.log("[API DEBUG] Response received:", {
				status: response.status,
				statusText: response.statusText,
			});
		}

		const responseEtag = response.headers.get("etag") ?? undefined;

		// Only a caller that offered a tag can interpret a 304; without one it would be an
		// unexpected empty success, so it keeps falling through to the error path below.
		if (response.status === 304 && ifNoneMatch) {
			return { notModified: true, etag: responseEtag ?? ifNoneMatch };
		}

		if (!response.ok) {
			if (response.status === 401 && auth) {
				requestSilentRenew(auth, "after 401");
			}
			const errorText = await response.text();
			const apiError = apiResponseError(response, errorText, path);
			console.error(`Error fetching ${path}:`, apiError.toJSON());
			throw apiError;
		}

		const text = await response.text();
		if (!text) return { notModified: false, etag: responseEtag };
		const json = tryParseJSON<T>(text);
		if (json === null) {
			return { notModified: false, data: text as T, etag: responseEtag };
		}
		if (API_STATS_ENABLED) {
			console.groupCollapsed(`API Request: ${path}`);
			console.dir(json, { depth: null });
			console.groupEnd();
		}
		return { notModified: false, data: json, etag: responseEtag };
	} catch (error) {
		if (error instanceof ApiResponseError) throw error;
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
