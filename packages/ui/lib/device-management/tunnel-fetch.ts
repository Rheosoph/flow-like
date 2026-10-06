import type { BrowserFetch } from "../service-runtime/transport";
import type { DeviceServiceStream } from "./tunnel";
import { TUNNEL_HTTP_BODY_LIMIT, tunnelHttpRequest } from "./tunnel-http";

export interface TunnelFetchOptions {
	open: (signal?: AbortSignal) => Promise<DeviceServiceStream>;
	/** Fixed HTTP Host. The authorized service ID determines the TCP destination. */
	authority?: string;
	signal?: AbortSignal;
	/** How long each request may wait for its response head; see `TunnelHttpRequest.headTimeoutMs`. */
	headTimeoutMs?: number;
}

/** A fetch implementation for one authorized service. Only origin-form paths are accepted. */
export function createTunnelFetch(options: TunnelFetchOptions): BrowserFetch {
	return async (input, init = {}) => {
		if (typeof input !== "string" || !validServicePath(input))
			throw new Error("Use a relative path within the deployed service.");
		if (init.credentials && init.credentials !== "omit")
			throw new Error("Device services do not use browser cookies.");
		if (
			init.redirect &&
			init.redirect !== "manual" &&
			init.redirect !== "error"
		)
			throw new Error("Device service redirects must be handled explicitly.");
		const controller = new AbortController();
		const signals = [options.signal, init.signal].filter(
			(signal): signal is AbortSignal => !!signal,
		);
		const abort = () => controller.abort();
		for (const signal of signals) {
			signal.addEventListener("abort", abort, { once: true });
			if (signal.aborted) abort();
		}
		const dispose = () => {
			for (const signal of signals) signal.removeEventListener("abort", abort);
		};
		let response: Awaited<ReturnType<typeof tunnelHttpRequest>> | undefined;
		try {
			controller.signal.throwIfAborted();
			const headers = new Headers(init.headers);
			const body = await requestBody(init.body, headers);
			controller.signal.throwIfAborted();
			const method = (init.method ?? "GET").toUpperCase();
			if ((method === "GET" || method === "HEAD") && body?.byteLength)
				throw new Error(`${method} service requests cannot contain a body.`);
			response = await tunnelHttpRequest(options.open, {
				method,
				path: input,
				authority: options.authority ?? "localhost",
				headers: Object.fromEntries(headers.entries()),
				body,
				signal: controller.signal,
				headTimeoutMs: options.headTimeoutMs,
			});
			if (
				init.redirect === "error" &&
				response.status >= 300 &&
				response.status < 400
			)
				throw new TypeError(
					"The deployed service attempted to redirect the request.",
				);
			const encoding = response.headers["content-encoding"];
			if (encoding && encoding.toLowerCase() !== "identity")
				throw new Error(
					"The device returned an unsupported compressed response.",
				);
			const responseHeaders = new Headers(response.headers);
			for (const name of [
				"connection",
				"transfer-encoding",
				"keep-alive",
				"trailer",
			])
				responseHeaders.delete(name);
			if (
				method === "HEAD" ||
				response.status === 204 ||
				response.status === 205 ||
				response.status === 304
			) {
				response.cancel();
				dispose();
				return new Response(null, {
					status: response.status,
					statusText: response.statusText,
					headers: responseHeaders,
				});
			}
			const result = response;
			const iterator = result.body[Symbol.asyncIterator]();
			let ended = false;
			let readable: ReadableStreamDefaultController<Uint8Array>;
			const finish = () => {
				if (ended) return;
				ended = true;
				controller.signal.removeEventListener("abort", cancelled);
				dispose();
			};
			const cancelled = () => {
				if (ended) return;
				finish();
				result.cancel();
				readable.error(
					new DOMException("The service request was cancelled.", "AbortError"),
				);
			};
			const stream = new ReadableStream<Uint8Array>(
				{
					start(target) {
						readable = target;
						controller.signal.addEventListener("abort", cancelled, {
							once: true,
						});
						if (controller.signal.aborted) cancelled();
					},
					async pull(target) {
						try {
							const next = await iterator.next();
							if (ended) return;
							if (next.done) {
								finish();
								target.close();
							} else target.enqueue(next.value);
						} catch (error) {
							if (ended) return;
							finish();
							result.cancel();
							target.error(error);
						}
					},
					cancel() {
						finish();
						result.cancel();
					},
				},
				{ highWaterMark: 0 },
			);
			return new Response(stream, {
				status: result.status,
				statusText: result.statusText,
				headers: responseHeaders,
			});
		} catch (error) {
			response?.cancel();
			controller.abort();
			dispose();
			throw error;
		}
	};
}

/** Reject normalization before handing a path to a scoped route or HTTP parser. */
export function validServicePath(path: string): boolean {
	if (
		!path.startsWith("/") ||
		path.startsWith("//") ||
		/[^\x21-\x7e]|[#\\]/.test(path)
	)
		return false;
	const pathname = path.split("?", 1)[0];
	try {
		return pathname
			.split("/")
			.slice(1)
			.every((part) => {
				const decoded = decodeURIComponent(part);
				return (
					decoded !== "." &&
					decoded !== ".." &&
					!/[%/\\]/.test(decoded) &&
					![...decoded].some(
						(character) =>
							character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
					)
				);
			});
	} catch {
		return false;
	}
}

async function requestBody(
	body: BodyInit | null | undefined,
	headers: Headers,
): Promise<Uint8Array | undefined> {
	if (body == null) return undefined;
	let bytes: Uint8Array;
	if (typeof body === "string") {
		if (body.length > TUNNEL_HTTP_BODY_LIMIT)
			throw new Error("Service request bodies are limited to 10 MiB.");
		bytes = new TextEncoder().encode(body);
		if (!headers.has("content-type"))
			headers.set("content-type", "text/plain;charset=UTF-8");
	} else if (body instanceof URLSearchParams) {
		bytes = new TextEncoder().encode(body.toString());
		if (!headers.has("content-type"))
			headers.set(
				"content-type",
				"application/x-www-form-urlencoded;charset=UTF-8",
			);
	} else if (body instanceof Blob) {
		if (body.size > TUNNEL_HTTP_BODY_LIMIT)
			throw new Error("Service request bodies are limited to 10 MiB.");
		if (body.type && !headers.has("content-type"))
			headers.set("content-type", body.type);
		bytes = new Uint8Array(await body.arrayBuffer());
	} else if (body instanceof ArrayBuffer) bytes = new Uint8Array(body);
	else if (ArrayBuffer.isView(body))
		bytes = new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
	else
		throw new Error(
			"Use a string, byte array, Blob, or File for a service request body.",
		);
	if (bytes.byteLength > TUNNEL_HTTP_BODY_LIMIT)
		throw new Error("Service request bodies are limited to 10 MiB.");
	return bytes;
}
