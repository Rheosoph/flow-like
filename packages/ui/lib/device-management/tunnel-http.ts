import type { DeviceServiceStream } from "./tunnel";
import { TUNNEL_MAX_DATA } from "./tunnel-protocol";

export const TUNNEL_HTTP_HEADER_LIMIT = 64 * 1024;
export const TUNNEL_HTTP_BODY_LIMIT = 10 * 1024 * 1024;
export const TUNNEL_HTTP_OPEN_TIMEOUT = 30_000;
export const TUNNEL_HTTP_HEADER_TIMEOUT = 60_000;
export const TUNNEL_HTTP_READ_TIMEOUT = 5 * 60_000;
export const TUNNEL_HTTP_HEAD_STALLED =
	"The service request or response headers stalled.";

export interface TunnelHttpRequest {
	method: string;
	path: string;
	/** HTTP Host only. The device chooses the configured service destination. */
	authority?: string;
	headers?: Record<string, string>;
	body?: string | Uint8Array;
	signal?: AbortSignal;
	/** How long the request and the response head may stall; `TUNNEL_HTTP_HEADER_TIMEOUT` otherwise. */
	headTimeoutMs?: number;
}

export interface TunnelHttpResponse {
	status: number;
	statusText: string;
	/** Lowercase names. Cookies are discarded; no browser cookie jar is involved. */
	headers: Record<string, string>;
	/** Consume once, or call cancel to release the service stream. */
	body: AsyncIterable<Uint8Array>;
	cancel(): void;
}

const token = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;
const forbidden = new Set([
	"host",
	"content-length",
	"transfer-encoding",
	"connection",
	"proxy-connection",
	"keep-alive",
	"te",
	"trailer",
	"upgrade",
	"expect",
	"accept-encoding",
	"cookie",
	"cookie2",
	"proxy-authorization",
	"proxy-authenticate",
]);
const invalid = (message: string) =>
	new Error(`Invalid service HTTP response: ${message}`);
const cancelled = () =>
	new DOMException("The service request was cancelled.", "AbortError");

function encodeRequest(input: TunnelHttpRequest) {
	const method = input.method;
	if (
		!token.test(method) ||
		method.length > 32 ||
		["CONNECT", "TRACE"].includes(method.toUpperCase())
	)
		throw new Error(
			"Use an HTTP API method such as GET, POST, PUT, PATCH, or DELETE.",
		);
	if (
		!input.path.startsWith("/") ||
		input.path.startsWith("//") ||
		/[^\x21-\x7e]|[#\\]/.test(input.path)
	)
		throw new Error(
			"Use an encoded service path starting with one slash, without a URL or fragment.",
		);
	const authority = input.authority ?? "localhost";
	if (
		!/^(?:[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?|\[[0-9A-Fa-f:.]+\])(?::[0-9]{1,5})?$/.test(
			authority,
		) ||
		authority.length > 255
	)
		throw new Error("The service HTTP authority is invalid.");
	const port = /:([0-9]+)$/.exec(authority)?.[1];
	if (port && (Number(port) < 1 || Number(port) > 65535))
		throw new Error("The service HTTP authority port is invalid.");
	if (
		typeof input.body === "string" &&
		input.body.length > TUNNEL_HTTP_BODY_LIMIT
	)
		throw new Error("Service request bodies are limited to 10 MiB.");
	const body =
		typeof input.body === "string"
			? new TextEncoder().encode(input.body)
			: (input.body ?? new Uint8Array());
	if (!(body instanceof Uint8Array) || body.length > TUNNEL_HTTP_BODY_LIMIT)
		throw new Error("Service request bodies are limited to 10 MiB.");
	const lines = [
		`${method} ${input.path} HTTP/1.1`,
		`Host: ${authority}`,
		"Connection: close",
		"Accept-Encoding: identity",
		`Content-Length: ${body.length}`,
	];
	const names = new Set<string>();
	for (const [name, value] of Object.entries(input.headers ?? {})) {
		const lower = name.toLowerCase();
		if (
			!token.test(name) ||
			forbidden.has(lower) ||
			names.has(lower) ||
			typeof value !== "string" ||
			/[^\t\x20-\x7e]/.test(value)
		)
			throw new Error(
				`The service request header ${name} is invalid or controlled by the tunnel.`,
			);
		names.add(lower);
		lines.push(`${name}: ${value}`);
	}
	const head = new TextEncoder().encode(`${lines.join("\r\n")}\r\n\r\n`);
	if (head.length > TUNNEL_HTTP_HEADER_LIMIT)
		throw new Error("Service request headers are limited to 64 KiB.");
	return { head, body };
}

class Exchange {
	readonly controller = new AbortController();
	stream?: DeviceServiceStream;
	private error?: Error;
	private done = false;
	private reject!: (error: Error) => void;
	private readonly failure = new Promise<never>((_, reject) => {
		this.reject = reject;
	});
	private timer?: ReturnType<typeof setTimeout>;
	private readonly onAbort = () => this.fail(cancelled());
	constructor(private readonly signal?: AbortSignal) {
		void this.failure.catch(() => {});
		signal?.addEventListener("abort", this.onAbort, { once: true });
		if (signal?.aborted) this.onAbort();
	}
	check() {
		if (this.error) throw this.error;
	}
	wait<T>(promise: Promise<T>): Promise<T> {
		this.check();
		return Promise.race([promise, this.failure]);
	}
	deadline(milliseconds: number, message: string) {
		clearTimeout(this.timer);
		if (!this.done)
			this.timer = setTimeout(
				() => this.fail(new Error(message)),
				milliseconds,
			);
	}
	clearDeadline() {
		clearTimeout(this.timer);
	}
	attach(stream: DeviceServiceStream) {
		if (this.done) {
			stream.reset();
			return;
		}
		this.stream = stream;
	}
	fail(error: Error) {
		if (this.done) return;
		this.error = error;
		this.reject(error);
		this.finish();
	}
	finish() {
		if (this.done) return;
		this.done = true;
		this.clearDeadline();
		this.signal?.removeEventListener("abort", this.onAbort);
		this.controller.abort();
		if (this.stream && !this.stream.finished) this.stream.reset();
	}
}

class Reader {
	private chunk: Uint8Array = new Uint8Array();
	private offset = 0;
	constructor(
		private readonly exchange: Exchange,
		private readonly stream: DeviceServiceStream,
	) {}
	async take(maximum = TUNNEL_MAX_DATA): Promise<Uint8Array | null> {
		this.exchange.check();
		if (this.offset === this.chunk.length) {
			const chunk = await this.exchange.wait(this.stream.read());
			if (chunk === null) return null;
			if (chunk.length === 0 || chunk.length > TUNNEL_MAX_DATA)
				throw invalid("invalid stream chunk size.");
			this.chunk = chunk;
			this.offset = 0;
		}
		const end = Math.min(this.offset + maximum, this.chunk.length);
		const result = this.chunk.subarray(this.offset, end);
		this.offset = end;
		return result;
	}
	async line(maximum: number): Promise<string> {
		const bytes = new Uint8Array(maximum);
		let length = 0;
		while (true) {
			const chunk = await this.take();
			if (!chunk) throw invalid("unexpected end of headers or chunk metadata.");
			for (let index = 0; index < chunk.length; index++) {
				if (length === maximum)
					throw invalid("headers or chunk metadata exceed the size limit.");
				const byte = chunk[index];
				bytes[length++] = byte;
				if (byte === 10) {
					if (length < 2 || bytes[length - 2] !== 13)
						throw invalid("lines must end with CRLF.");
					this.offset -= chunk.length - index - 1;
					let result = "";
					for (let start = 0; start < length - 2; start += 4096)
						result += String.fromCharCode(
							...bytes.subarray(start, Math.min(start + 4096, length - 2)),
						);
					return result;
				}
				if (length > 1 && bytes[length - 2] === 13)
					throw invalid("bare carriage return.");
			}
		}
	}
}

function field(line: string): [string, string] {
	const separator = line.indexOf(":");
	const name = line.slice(0, separator);
	const value = line.slice(separator + 1).replace(/^[ \t]+|[ \t]+$/g, "");
	if (separator < 1 || !token.test(name) || /[^\t\x20-\xff]|\x7f/.test(value))
		throw invalid("malformed header field.");
	return [name.toLowerCase(), value];
}

type ResponseHead = {
	status: number;
	statusText: string;
	headers: Record<string, string>;
	length?: number;
	chunked: boolean;
};
async function readHead(reader: Reader): Promise<ResponseHead> {
	let total = 0;
	for (let interim = 0; interim <= 8; interim++) {
		const statusLine = await reader.line(TUNNEL_HTTP_HEADER_LIMIT - total);
		total += statusLine.length + 2;
		const statusMatch =
			/^HTTP\/1\.[01] ([1-5][0-9]{2})(?: ([\t\x20-\x7e\x80-\xff]*))?$/.exec(
				statusLine,
			);
		if (!statusMatch) throw invalid("malformed status line.");
		const status = Number(statusMatch[1]);
		const headers: Record<string, string> = Object.create(null);
		let count = 0;
		while (true) {
			const line = await reader.line(TUNNEL_HTTP_HEADER_LIMIT - total);
			total += line.length + 2;
			if (!line) break;
			if (++count > 256) throw invalid("too many header fields.");
			const [name, value] = field(line);
			if (
				(name === "content-length" || name === "transfer-encoding") &&
				Object.hasOwn(headers, name)
			)
				throw invalid("duplicate body framing headers.");
			if (name === "set-cookie" || name === "set-cookie2") continue;
			headers[name] = Object.hasOwn(headers, name)
				? `${headers[name]}, ${value}`
				: value;
		}
		const lengthValue = headers["content-length"];
		const encoding = headers["transfer-encoding"];
		if (
			lengthValue !== undefined &&
			(!/^[0-9]+$/.test(lengthValue) ||
				!Number.isSafeInteger(Number(lengthValue)))
		)
			throw invalid("invalid Content-Length.");
		if (
			encoding !== undefined &&
			(encoding.toLowerCase() !== "chunked" || lengthValue !== undefined)
		)
			throw invalid("unsupported or conflicting transfer framing.");
		if (status === 101 || headers.upgrade !== undefined)
			throw invalid("protocol upgrades are unsupported.");
		if (status < 200) {
			if (encoding !== undefined || lengthValue !== undefined)
				throw invalid("interim responses cannot frame a body.");
			continue;
		}
		if (status === 204 && (lengthValue !== undefined || encoding !== undefined))
			throw invalid("a 204 response cannot frame a body.");
		return {
			status,
			statusText: statusMatch[2] ?? "",
			headers,
			length: lengthValue === undefined ? undefined : Number(lengthValue),
			chunked: encoding !== undefined,
		};
	}
	throw invalid("too many interim responses.");
}

function chunkLength(line: string): number {
	const match = /^([0-9a-fA-F]+)(.*)$/.exec(line);
	if (!match) throw invalid("malformed chunk length.");
	const length = Number.parseInt(match[1], 16);
	if (!Number.isSafeInteger(length))
		throw invalid("chunk length is too large.");
	let extensions = match[2];
	const extension =
		/^[ \t]*;[ \t]*[!#$%&'*+.^_`|~0-9A-Za-z-]+(?:[ \t]*=[ \t]*(?:[!#$%&'*+.^_`|~0-9A-Za-z-]+|"(?:[\t\x20\x21\x23-\x5b\x5d-\x7e\x80-\xff]|\\[\t\x20-\x7e\x80-\xff])*"))?/;
	while (extensions) {
		const next = extension.exec(extensions);
		if (!next) throw invalid("malformed chunk extension.");
		extensions = extensions.slice(next[0].length);
	}
	return length;
}

async function* decodeBody(
	reader: Reader,
	head: ResponseHead,
	method: string,
): AsyncGenerator<Uint8Array> {
	if (method === "HEAD" || head.status === 204 || head.status === 304) return;
	if (head.chunked) {
		while (true) {
			let remaining = chunkLength(await reader.line(8192));
			if (remaining === 0) {
				let total = 0;
				let count = 0;
				while (true) {
					const line = await reader.line(TUNNEL_HTTP_HEADER_LIMIT - total);
					total += line.length + 2;
					if (!line) return;
					if (++count > 256) throw invalid("too many trailer fields.");
					const [name] = field(line);
					if (
						forbidden.has(name) ||
						["content-encoding", "content-type", "content-range"].includes(name)
					)
						throw invalid("forbidden trailer field.");
				}
			}
			while (remaining > 0) {
				const chunk = await reader.take(remaining);
				if (!chunk) throw invalid("truncated chunk body.");
				remaining -= chunk.length;
				yield chunk;
			}
			if ((await reader.line(2)) !== "")
				throw invalid("missing chunk terminator.");
		}
	}
	let remaining = head.length ?? Number.POSITIVE_INFINITY;
	while (remaining > 0) {
		const chunk = await reader.take(remaining);
		if (!chunk) {
			if (Number.isFinite(remaining))
				throw invalid("truncated Content-Length body.");
			return;
		}
		remaining -= chunk.length;
		yield chunk;
	}
}

/** One HTTP/1.1 exchange over an authorized service stream. Redirects remain responses. */
export async function tunnelHttpRequest(
	open: (signal?: AbortSignal) => Promise<DeviceServiceStream>,
	input: TunnelHttpRequest,
): Promise<TunnelHttpResponse> {
	const request = encodeRequest(input);
	const exchange = new Exchange(input.signal);
	let finalHead = false;
	try {
		exchange.check();
		exchange.deadline(
			TUNNEL_HTTP_OPEN_TIMEOUT,
			"The service connection timed out.",
		);
		const opening = open(exchange.controller.signal).then((stream) => {
			exchange.attach(stream);
			return stream;
		});
		const stream = await exchange.wait(opening);
		const progress = () => {
			if (!finalHead)
				exchange.deadline(
					input.headTimeoutMs ?? TUNNEL_HTTP_HEADER_TIMEOUT,
					TUNNEL_HTTP_HEAD_STALLED,
				);
		};
		progress();
		const write = async () => {
			for (const part of [request.head, request.body]) {
				for (
					let offset = 0;
					offset < part.length && !finalHead;
					offset += TUNNEL_MAX_DATA
				)
					await exchange.wait(
						stream.write(
							part.subarray(offset, offset + TUNNEL_MAX_DATA),
							progress,
						),
					);
			}
		};
		void write().catch((error) => {
			if (!finalHead)
				exchange.fail(
					error instanceof Error ? error : new Error(String(error)),
				);
		});
		const reader = new Reader(exchange, stream);
		const head = await readHead(reader);
		finalHead = true;
		exchange.clearDeadline();
		let consumed = false;
		const body: AsyncIterable<Uint8Array> = {
			[Symbol.asyncIterator]() {
				if (consumed)
					throw new Error(
						"The service response body can only be consumed once.",
					);
				consumed = true;
				const source = decodeBody(reader, head, input.method);
				return {
					async next() {
						try {
							exchange.check();
							exchange.deadline(
								TUNNEL_HTTP_READ_TIMEOUT,
								"The service response body stalled.",
							);
							const result = await exchange.wait(source.next());
							exchange.clearDeadline();
							if (result.done) exchange.finish();
							return result;
						} catch (error) {
							exchange.fail(
								error instanceof Error ? error : new Error(String(error)),
							);
							throw error;
						}
					},
					async return() {
						exchange.finish();
						return { done: true as const, value: undefined };
					},
				};
			},
		};
		return {
			status: head.status,
			statusText: head.statusText,
			headers: head.headers,
			body,
			cancel: () => exchange.fail(cancelled()),
		};
	} catch (error) {
		exchange.fail(error instanceof Error ? error : new Error(String(error)));
		throw error;
	}
}
