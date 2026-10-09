import { buildAuthHeaders } from "./auth.js";
import { AuthError, FlowLikeError, NotFoundError } from "./errors.js";
import type { AuthConfig } from "./types.js";

export function stripTrailingSlashes(url: string): string {
	let i = url.length;
	while (i > 0 && url[i - 1] === "/") i--;
	return url.slice(0, i);
}

export interface HttpClient {
	request<T = unknown>(
		method: string,
		path: string,
		options?: RequestOptions,
	): Promise<T>;
	requestRaw(
		method: string,
		path: string,
		options?: RequestOptions,
	): Promise<Response>;
	streamSSE(
		method: string,
		path: string,
		options?: RequestOptions,
	): AsyncIterable<SSEChunk>;
}

export interface RequestOptions {
	body?: unknown;
	headers?: Record<string, string>;
	signal?: AbortSignal;
	query?: QueryParams;
	/** Use a run-specific bearer token without forwarding platform credentials. */
	auth?: false;
}

export type QueryParams = Record<string, string | number | boolean | undefined>;

export interface SSEChunk {
	event?: string;
	data: string;
	id?: string;
}

function buildQueryString(params?: QueryParams): string {
	if (!params) return "";
	const entries = Object.entries(params).filter(([, v]) => v !== undefined) as [
		string,
		string | number | boolean,
	][];
	if (entries.length === 0) return "";
	const qs = new URLSearchParams(
		entries.map(([k, v]) => [k, String(v)] as [string, string]),
	).toString();
	return `?${qs}`;
}

async function handleErrorResponse(res: Response): Promise<never> {
	const raw = await res.text();
	let body: unknown = raw;
	try {
		body = JSON.parse(raw);
	} catch {
		/* Providers may return plain text. */
	}
	const record =
		typeof body === "object" && body !== null
			? (body as Record<string, unknown>)
			: undefined;
	const detail = record?.error;
	const message = String(
		record?.message ??
			(typeof detail === "object" && detail !== null
				? (detail as Record<string, unknown>).message
				: detail) ??
			(raw || `HTTP ${res.status}: ${res.statusText}`),
	);
	if (res.status === 401 || res.status === 403)
		throw new AuthError(message, res.status, body);
	if (res.status === 404) throw new NotFoundError(message, body);
	throw new FlowLikeError(message, res.status, body);
}

/** Preserve deployment prefixes and accept either an origin or its /api/v1 endpoint. */
export function normalizeBaseUrl(baseUrl: string): string {
	const url = new URL(baseUrl);
	if (
		!["http:", "https:"].includes(url.protocol) ||
		url.username ||
		url.password ||
		url.search ||
		url.hash
	) {
		throw new FlowLikeError(
			"Base URL must be HTTP(S) without credentials, query, or fragment",
		);
	}
	const base = stripTrailingSlashes(url.toString());
	return base.endsWith("/api/v1") ? base.slice(0, -7) : base;
}

export function createHttpClient(
	baseUrl: string,
	auth: AuthConfig,
): HttpClient {
	const authHeaders = buildAuthHeaders(auth);
	const base = normalizeBaseUrl(baseUrl);

	async function doFetch(
		method: string,
		path: string,
		options?: RequestOptions,
	): Promise<Response> {
		if (
			!path.startsWith("/") ||
			path.startsWith("//") ||
			path.includes("#") ||
			path.includes("?") ||
			path.includes("\\") ||
			path.split("/").some((part) => /^(?:\.|%2e){1,2}$/i.test(part))
		) {
			throw new FlowLikeError(
				"API paths must be relative to /api/v1; use query options for parameters",
			);
		}
		const url = `${base}/api/v1${path}${buildQueryString(options?.query)}`;
		const headers = new Headers(options?.auth === false ? {} : authHeaders);
		headers.set("X-Flow-Like-Board-Format", "2");
		for (const [key, value] of Object.entries(options?.headers ?? {}))
			headers.set(key, value);

		let fetchBody: string | FormData | undefined;
		if (options?.body instanceof FormData) {
			fetchBody = options.body;
		} else if (options?.body !== undefined) {
			headers.set("Content-Type", "application/json");
			fetchBody = JSON.stringify(options.body);
		}

		const res = await fetch(url, {
			method,
			headers,
			body: fetchBody,
			signal: options?.signal,
			redirect: "error",
			credentials: "omit",
		});

		return res;
	}

	return {
		async request<T = unknown>(
			method: string,
			path: string,
			options?: RequestOptions,
		): Promise<T> {
			const res = await doFetch(method, path, options);
			if (!res.ok) await handleErrorResponse(res);
			if (res.status === 204) return undefined as T;
			const text = await res.text();
			return (text ? JSON.parse(text) : undefined) as T;
		},

		async requestRaw(
			method: string,
			path: string,
			options?: RequestOptions,
		): Promise<Response> {
			const res = await doFetch(method, path, options);
			if (!res.ok) await handleErrorResponse(res);
			return res;
		},

		async *streamSSE(
			method: string,
			path: string,
			options?: RequestOptions,
		): AsyncIterable<SSEChunk> {
			const res = await doFetch(method, path, {
				...options,
				headers: { ...options?.headers, Accept: "text/event-stream" },
			});
			if (!res.ok) await handleErrorResponse(res);
			if (
				res.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !==
				"text/event-stream"
			) {
				await res.body?.cancel().catch(() => undefined);
				throw new FlowLikeError("Expected text/event-stream response");
			}
			if (!res.body) throw new FlowLikeError("No response body for SSE stream");

			const reader = res.body.getReader();
			const decoder = new TextDecoder();
			let buffer = "";

			try {
				while (true) {
					const { done, value } = await reader.read();
					if (done) {
						buffer += decoder.decode();
						break;
					}

					buffer += decoder.decode(value, { stream: true });
					const parts = buffer.split(/\r\n\r\n|\n\n|\r\r/);
					buffer = parts.pop() ?? "";

					for (const part of parts) {
						const chunk = parseSSEBlock(part);
						if (chunk) yield chunk;
					}
				}

				if (buffer.trim()) {
					const chunk = parseSSEBlock(buffer);
					if (chunk) yield chunk;
				}
			} finally {
				await reader.cancel().catch(() => undefined);
				reader.releaseLock();
			}
		},
	};
}

function parseSSEBlock(block: string): SSEChunk | null {
	const lines = block.split(/\r\n|\r|\n/);
	let event: string | undefined;
	const data: string[] = [];
	let id: string | undefined;

	for (const line of lines) {
		if (line.startsWith("event:")) {
			event = line.slice(6).replace(/^ /, "");
		} else if (line.startsWith("data:")) {
			data.push(line.slice(5).replace(/^ /, ""));
		} else if (line.startsWith("id:")) {
			id = line.slice(3).replace(/^ /, "");
		}
	}

	if (data.length === 0 && !event) return null;
	return { event, data: data.join("\n"), id };
}
