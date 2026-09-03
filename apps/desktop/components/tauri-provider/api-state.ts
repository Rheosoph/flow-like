import {
	type IApiState,
	type IProfile,
	getActiveTraceContext,
	getTelemetryTraceparent,
} from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { type EventSourceMessage, createEventSource } from "eventsource-client";
import type { AuthContextProps } from "react-oidc-context";
import { ensureProtectedAppRouteAuth, requestSilentRenew } from "../../lib/api";
import { apiResponseError } from "../../lib/api-error";
import {
	DEFAULT_CONNECT_TIMEOUT_MS,
	STREAM_HEADER_TIMEOUT_MS,
	withRequestDeadline,
} from "../../lib/request-deadline";

function constructUrl(profile: IProfile, path: string): string {
	return getApiUrl(profile, path);
}

type SSEMessage = {
	event?: string;
	data: string;
	id?: string;
	raw: string;
};

type ProcessedSSEEvent = {
	type: string;
	error?: Error;
};

/**
 * W3C trace propagation for outgoing API calls. Empty when no trace is active
 * or the active trace was not sampled, so tracing stays free when it is off.
 */
function traceHeaders(): Record<string, string> {
	try {
		const context = getActiveTraceContext();
		if (!context?.sampled) return {};
		const traceparent = getTelemetryTraceparent(context);
		return traceparent ? { traceparent } : {};
	} catch {
		return {};
	}
}

function tryParseJSON<T>(text: string): T | null {
	try {
		return JSON.parse(text) as T;
	} catch {
		return null;
	}
}

function buildSSEError(
	event: SSEMessage,
	parsedData: Record<string, unknown> | null,
): Error {
	const message =
		typeof parsedData?.message === "string"
			? parsedData.message
			: typeof parsedData?.error === "string"
				? parsedData.error
				: event.data || "SSE stream error";

	return new Error(message);
}

function parseSSEBuffer(buffer: string): {
	events: SSEMessage[];
	remaining: string;
} {
	const events: SSEMessage[] = [];
	const parts = buffer.split("\n\n");
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
				continue;
			}
		}

		if (data) {
			events.push({ event, data, id, raw: part });
		}
	}

	return { events, remaining };
}

function processSSEEvent<T>(
	event: SSEMessage,
	onMessage?: (data: T) => void,
): ProcessedSSEEvent {
	const evt = event.event ?? "message";
	const parsedData = tryParseJSON<T>(event.data);
	if (parsedData && onMessage) {
		onMessage(parsedData);
	}

	const data = parsedData as Record<string, unknown> | null;
	const eventType = data?.event_type ?? data?.type;
	if (evt === "done" || evt === "completed" || eventType === "completed") {
		return { type: "completed" };
	}
	if (evt === "error" || eventType === "error") {
		return {
			type: "error",
			error: buildSSEError(event, data),
		};
	}
	return { type: evt };
}

export class TauriApiState implements IApiState {
	private auth: AuthContextProps | null = null;

	setAuth(auth: AuthContextProps | null) {
		this.auth = auth;
	}

	private getAuthHeader(): Record<string, string> {
		return this.auth?.user?.access_token
			? { Authorization: `Bearer ${this.auth.user.access_token}` }
			: {};
	}

	async fetch<T>(
		profile: IProfile,
		path: string,
		options?: RequestInit,
	): Promise<T> {
		ensureProtectedAppRouteAuth(
			path,
			this.auth,
			(options?.method ?? "GET").toUpperCase(),
		);
		const url = constructUrl(profile, path);
		const authHeader = this.getAuthHeader();

		if (typeof navigator !== "undefined" && !navigator.onLine) {
			throw new Error(`Network unavailable: ${path}`);
		}

		try {
			// The deadline spans the body read as well: `fetch_read_body` stalls on a
			// half-open socket exactly like `fetch_send` does.
			return await withRequestDeadline<T>(
				path,
				async ({ signal }) => {
					const response = await tauriFetch(url, {
						...options,
						headers: {
							"Content-Type": "application/json",
							...traceHeaders(),
							...options?.headers,
							...authHeader,
						},
						keepalive: true,
						priority: "high",
						connectTimeout: DEFAULT_CONNECT_TIMEOUT_MS,
						signal,
					});

					if (!response.ok) {
						if (response.status === 401 && this.auth) {
							requestSilentRenew(this.auth, "after 401");
						}
						const errorText = await response.text();
						throw apiResponseError(response, errorText, path);
					}

					if (response.status === 204) return undefined as T;

					return (await response.json()) as T;
				},
				{ signal: options?.signal },
			);
		} catch (error) {
			if (error instanceof Error) {
				if (
					error.message.includes("Failed to fetch") ||
					error.message.includes("NetworkError") ||
					error.message.includes("Network request failed") ||
					error.message.includes("fetch failed")
				) {
					throw new Error(`Network unavailable: ${path}`);
				}
			}
			if (error instanceof Error) throw error;
			throw new Error(`Error fetching data: ${String(error)}`);
		}
	}

	async get<T>(profile: IProfile, path: string): Promise<T> {
		return this.fetch<T>(profile, path, { method: "GET" });
	}

	async post<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "POST",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async put<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "PUT",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async patch<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "PATCH",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async del<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "DELETE",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async stream<T>(
		profile: IProfile,
		path: string,
		options?: RequestInit,
		onMessage?: (data: T) => void,
	): Promise<void> {
		const url = constructUrl(profile, path);
		const authHeader = this.getAuthHeader();
		const method = options?.method ?? "GET";

		if (method === "POST" || method === "PUT") {
			await this.streamRaw<T>(url, options, authHeader, onMessage);
			return;
		}

		await this.streamEventSource<T>(url, options, authHeader, onMessage);
	}

	private async streamRaw<T>(
		url: string,
		options: RequestInit | undefined,
		authHeader: Record<string, string>,
		onMessage?: (data: T) => void,
	): Promise<void> {
		const abortController = new AbortController();
		// Bounded until headers arrive, then released — the stream itself is
		// long-lived by design and must not be cut off by a deadline. The reader
		// below owns `abortController` and terminates the stream through it.
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
			if (response.status === 401 && this.auth) {
				requestSilentRenew(this.auth, "after 401");
			}
			const errorText = await response.text();
			throw apiResponseError(response, errorText, url);
		}

		if (!response.body) {
			throw new Error("Response body is null - streaming not supported");
		}

		const reader = response.body.getReader();
		const decoder = new TextDecoder();
		let buffer = "";

		try {
			while (true) {
				const { done, value } = await reader.read();

				if (done) {
					if (buffer.trim()) {
						const { events } = parseSSEBuffer(buffer + "\n\n");
						for (const event of events) {
							const result = processSSEEvent(event, onMessage);
							if (result.type === "error") {
								throw result.error ?? new Error("SSE stream error");
							}
							if (result.type === "completed") {
								return;
							}
						}
					}
					break;
				}

				buffer += decoder.decode(value, { stream: true });
				const { events, remaining } = parseSSEBuffer(buffer);
				buffer = remaining;

				for (const event of events) {
					const result = processSSEEvent(event, onMessage);
					if (result.type === "error") {
						abortController.abort();
						throw result.error ?? new Error("SSE stream error");
					}
					if (result.type === "completed") {
						abortController.abort();
						return;
					}
				}
			}
		} finally {
			try {
				reader.releaseLock();
			} catch {
				// Ignore
			}
		}
	}

	private async streamEventSource<T>(
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
					const result = processSSEEvent<T>(
						{
							event: message.event,
							data: message.data,
							id: message.id,
							raw: message.data,
						},
						onMessage,
					);

					if (result.type === "completed") {
						closeAndResolve();
					}
					if (result.type === "error") {
						closeAndReject(result.error ?? new Error("SSE stream error"));
					}
				},
				onConnect: () => {},
				onScheduleReconnect: () => {
					closeAndResolve();
				},
				onDisconnect: () => {
					closeAndResolve();
				},
				onError: (error: unknown) => {
					closeAndReject(
						error instanceof Error ? error : new Error(String(error)),
					);
				},
			});
		});
	}
}
