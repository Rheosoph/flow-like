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
	query?: Record<string, string | number | undefined>;
}

export interface SSEChunk {
	event?: string;
	data: string;
	id?: string;
}

function buildQueryString(
	params?: Record<string, string | number | undefined>,
): string {
	if (!params) return "";
	const entries = Object.entries(params).filter(([, v]) => v !== undefined) as [
		string,
		string | number,
	][];
	if (entries.length === 0) return "";
	const qs = new URLSearchParams(
		entries.map(([k, v]) => [k, String(v)] as [string, string]),
	).toString();
	return `?${qs}`;
}

async function handleErrorResponse(res: Response): Promise<never> {
	let body: unknown;
	try {
		body = await res.json();
	} catch {
		body = await res.text().catch(() => undefined);
	}

	const message =
		typeof body === "object" && body !== null && "message" in body
			? String((body as { message: string }).message)
			: `HTTP ${res.status}: ${res.statusText}`;

	if (res.status === 401 || res.status === 403) throw new AuthError(message);
	if (res.status === 404) throw new NotFoundError(message);
	throw new FlowLikeError(message, res.status, body);
}

export function createHttpClient(
	baseUrl: string,
	auth: AuthConfig,
): HttpClient {
	const authHeaders = buildAuthHeaders(auth);
	const base = stripTrailingSlashes(baseUrl);

	async function doFetch(
		method: string,
		path: string,
		options?: RequestOptions,
	): Promise<Response> {
		const url = `${base}/api/v1${path}${buildQueryString(options?.query)}`;
		const headers: Record<string, string> = {
			...authHeaders,
			...options?.headers,
		};

		let fetchBody: string | FormData | undefined;
		if (options?.body instanceof FormData) {
			fetchBody = options.body;
		} else if (options?.body !== undefined) {
			headers["Content-Type"] = "application/json";
			fetchBody = JSON.stringify(options.body);
		}

		const res = await fetch(url, {
			method,
			headers,
			body: fetchBody,
			signal: options?.signal,
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
			return (await res.json()) as T;
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
			if (!res.body) throw new FlowLikeError("No response body for SSE stream");

			const reader = res.body.getReader();
			const decoder = new TextDecoder();
			let buffer = "";

			try {
				while (true) {
					const { done, value } = await reader.read();
					if (done) break;

					buffer += decoder.decode(value, { stream: true });
					const parts = buffer.split("\n\n");
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
				reader.releaseLock();
			}
		},
	};
}

function parseSSEBlock(block: string): SSEChunk | null {
	const lines = block.split("\n");
	let event: string | undefined;
	let data = "";
	let id: string | undefined;

	for (const line of lines) {
		if (line.startsWith("event:")) {
			event = line.slice(6).trim();
		} else if (line.startsWith("data:")) {
			data += (data ? "\n" : "") + line.slice(5).trim();
		} else if (line.startsWith("id:")) {
			id = line.slice(3).trim();
		}
	}

	if (!data && !event) return null;
	return { event, data, id };
}
