import { afterEach, describe, expect, test } from "bun:test";
import { createServer } from "node:http";
import { connect } from "node:net";
import { FrameQueue } from "./transport";
import { type DeviceServiceStream, DeviceServiceTunnel } from "./tunnel";
import {
	TUNNEL_HTTP_BODY_LIMIT,
	TUNNEL_HTTP_HEADER_LIMIT,
	TUNNEL_HTTP_OPEN_TIMEOUT,
	TUNNEL_HTTP_READ_TIMEOUT,
	type TunnelHttpRequest,
	type TunnelHttpResponse,
	tunnelHttpRequest,
} from "./tunnel-http";
import {
	TUNNEL_MAX_DATA,
	type TunnelFrame,
	TunnelKind,
	decodeTunnelFrame,
	encodeTunnelFrame,
} from "./tunnel-protocol";
import type { TunnelTransport } from "./tunnel-transport";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const active: DeviceServiceTunnel[] = [];
const cancellations: (() => void)[] = [];
afterEach(() => {
	for (const cancel of cancellations.splice(0)) cancel();
	for (const tunnel of active.splice(0)) tunnel.close();
});
const settle = async () => {
	for (let index = 0; index < 40; index++) await Promise.resolve();
};
async function text(response: TunnelHttpResponse) {
	let result = "";
	for await (const chunk of response.body)
		result += decoder.decode(chunk, { stream: true });
	return result + decoder.decode();
}

class Peer implements TunnelTransport {
	kind = "websocket" as const;
	expiresAt = Math.floor(Date.now() / 1000) + 300;
	input = new FrameQueue<Uint8Array>(100_000);
	sequence = 1n;
	sent: TunnelFrame[] = [];
	id = 0;
	onData: (bytes: Uint8Array) => void = () => {};
	onReset: () => void = () => {};
	credit = true;
	async send(bytes: Uint8Array) {
		const frame = decodeTunnelFrame(bytes);
		this.sent.push({ ...frame, body: frame.body.slice() });
		if (frame.kind === TunnelKind.Open) {
			this.id = frame.stream;
			this.emit(TunnelKind.Opened);
		} else if (frame.kind === TunnelKind.Data) {
			this.onData(frame.body.slice());
			if (this.credit) {
				const window = new Uint8Array(4);
				new DataView(window.buffer).setUint32(0, frame.body.length);
				this.emit(TunnelKind.Window, window);
			}
		} else if (frame.kind === TunnelKind.Reset) this.onReset();
	}
	next() {
		return this.input.next();
	}
	emit(kind: TunnelKind, body: Uint8Array = new Uint8Array()) {
		this.input.push(
			encodeTunnelFrame({
				kind,
				body,
				stream: this.id,
				sequence: this.sequence++,
			}),
		);
	}
	respond(value: string, split = TUNNEL_MAX_DATA, fin = true) {
		const bytes = encoder.encode(value);
		for (let offset = 0; offset < bytes.length; offset += split)
			this.emit(TunnelKind.Data, bytes.slice(offset, offset + split));
		if (fin) this.emit(TunnelKind.Fin);
	}
	beginRenewal(): never {
		throw new Error("Unexpected renewal in short HTTP fixture.");
	}
	installSession() {}
	confirmRenewal() {}
	close() {
		this.input.close();
	}
	connect() {
		const tunnel = DeviceServiceTunnel.fromTransport(this);
		active.push(tunnel);
		return tunnel;
	}
}

async function requestResponse(
	raw: string,
	input: Partial<TunnelHttpRequest> = {},
	split = TUNNEL_MAX_DATA,
) {
	const peer = new Peer();
	const tunnel = peer.connect();
	let responded = false;
	peer.onData = () => {
		if (responded) return;
		responded = true;
		peer.respond(raw, split);
	};
	const response = await tunnelHttpRequest(
		(signal) => tunnel.open("placement", "hosting", { signal }),
		{ method: "GET", path: "/api", ...input },
	);
	cancellations.push(response.cancel);
	return { peer, response };
}

describe("HTTP over device service streams", () => {
	test("uses the real mux, encodes UTF-8 body length and returns streamed bytes", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		let request = "";
		peer.onData = (bytes) => {
			request += decoder.decode(bytes);
			if (request.endsWith("héllo"))
				peer.respond(
					"HTTP/1.1 201 Created\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello",
					1,
				);
		};
		const response = await tunnelHttpRequest(
			(signal) => tunnel.open("p", "hosting", { signal }),
			{
				method: "POST",
				path: "/api?query=%20",
				authority: "localhost:8080",
				headers: { Authorization: "Bearer token" },
				body: "héllo",
			},
		);
		expect(request).toContain(
			"POST /api?query=%20 HTTP/1.1\r\nHost: localhost:8080\r\n",
		);
		expect(request).toContain("Content-Length: 6\r\n");
		expect(request).toContain("Authorization: Bearer token\r\n");
		expect(response.status).toBe(201);
		expect(response.statusText).toBe("Created");
		expect(response.headers["content-type"]).toBe("text/plain");
		expect(await text(response)).toBe("hello");
		await settle();
		expect(peer.sent.some((frame) => frame.kind === TunnelKind.Reset)).toBe(
			true,
		);
	});

	test("decodes split chunk framing, extensions, trailers and interim responses", async () => {
		const { response } = await requestResponse(
			'HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </app>\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2; type="a\\"b"\r\nhe\r\n3 ;foo=bar\r\nllo\r\n0\r\nX-Checksum: okay\r\n\r\n',
			{},
			1,
		);
		expect(await text(response)).toBe("hello");
	});

	test("supports EOF bodies and returns redirects without cookies or following them", async () => {
		const { response, peer } = await requestResponse(
			"HTTP/1.0 302 Found\r\nLocation: https://example.com/\r\nSet-Cookie: token=secret\r\n\r\nmoved",
		);
		expect(response.status).toBe(302);
		expect(response.headers.location).toBe("https://example.com/");
		expect(response.headers["set-cookie"]).toBeUndefined();
		expect(await text(response)).toBe("moved");
		expect(
			peer.sent.filter((frame) => frame.kind === TunnelKind.Open),
		).toHaveLength(1);
	});

	for (const [method, status, extra] of [
		["HEAD", 200, "Content-Length: 42\r\n"],
		["GET", 204, ""],
		["GET", 304, "Content-Length: 42\r\n"],
	] as const)
		test(`${method} ${status} has no response body`, async () => {
			const { response } = await requestResponse(
				`HTTP/1.1 ${status} Okay\r\n${extra}\r\n`,
				{ method },
			);
			expect(await text(response)).toBe("");
		});

	test("keeps SSE duplex until the reader cancels and does not read ahead", async () => {
		const peer = new Peer();
		const tunnel = peer.connect();
		peer.onData = () =>
			peer.respond(
				"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: first\n\n",
				TUNNEL_MAX_DATA,
				false,
			);
		const response = await tunnelHttpRequest(
			(signal) => tunnel.open("p", "hosting", { signal }),
			{ method: "GET", path: "/events" },
		);
		const iterator = response.body[Symbol.asyncIterator]();
		expect(decoder.decode((await iterator.next()).value)).toBe(
			"data: first\n\n",
		);
		await settle();
		expect(
			peer.sent.some(
				(frame) =>
					frame.kind === TunnelKind.Fin || frame.kind === TunnelKind.Reset,
			),
		).toBe(false);
		peer.respond("data: second\n\n", TUNNEL_MAX_DATA, false);
		await settle();
		const creditBeforeRead = peer.sent.filter(
			(frame) => frame.kind === TunnelKind.Window,
		).length;
		expect(decoder.decode((await iterator.next()).value)).toBe(
			"data: second\n\n",
		);
		await settle();
		expect(
			peer.sent.filter((frame) => frame.kind === TunnelKind.Window).length,
		).toBeGreaterThan(creditBeforeRead);
		const pending = iterator.next();
		response.cancel();
		await expect(pending).rejects.toThrow("cancelled");
	});

	test("breaking response iteration releases the service stream", async () => {
		const { response, peer } = await requestResponse(
			"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\na",
		);
		for await (const _ of response.body) break;
		await settle();
		expect(peer.sent.some((frame) => frame.kind === TunnelKind.Reset)).toBe(
			true,
		);
		expect(() => response.body[Symbol.asyncIterator]()).toThrow(
			"only be consumed once",
		);
	});

	test("an early final response stops a backpressured request upload", async () => {
		const peer = new Peer();
		peer.credit = false;
		const tunnel = peer.connect();
		let received = 0;
		peer.onData = (bytes) => {
			received += bytes.length;
			if (received > 200_000)
				peer.respond("HTTP/1.1 413 Too Large\r\nContent-Length: 2\r\n\r\nno");
		};
		const response = await tunnelHttpRequest(
			(signal) => tunnel.open("p", "hosting", { signal }),
			{ method: "POST", path: "/", body: new Uint8Array(1_000_000) },
		);
		expect(response.status).toBe(413);
		expect(await text(response)).toBe("no");
		expect(received).toBeLessThan(300_000);
	});
});

describe("HTTP validation and cancellation", () => {
	for (const input of [
		{ path: "https://example.com/" },
		{ path: "//example.com/" },
		{ path: "/a\r\nHost: other" },
		{ path: "/fragment#x" },
		{ path: "/\\other" },
		{ method: "CONNECT" },
		{ method: "GET\r\n" },
		{ authority: "user@host" },
		{ authority: "localhost:99999" },
		{ headers: { Host: "other" } },
		{ headers: { "Content-Length": "1" } },
		{ headers: { Connection: "keep-alive" } },
		{ headers: { "Transfer-Encoding": "chunked" } },
		{ headers: { Expect: "100-continue" } },
		{ headers: { Cookie: "secret=value" } },
		{ headers: { Authorization: "Bearer x\r\nInjected: true" } },
		{ headers: { "invalid name": "x" } },
		{ headers: { "X-Token": "x", "x-token": "y" } },
		{ body: new Uint8Array(TUNNEL_HTTP_BODY_LIMIT + 1) },
	] as Partial<TunnelHttpRequest>[])
		test(`rejects invalid request before opening: ${JSON.stringify(input, (_, value) => (value instanceof Uint8Array ? value.length : value))}`, async () => {
			let opened = false;
			await expect(
				tunnelHttpRequest(
					async () => {
						opened = true;
						throw new Error("must not open");
					},
					{ method: "GET", path: "/", ...input },
				),
			).rejects.toThrow();
			expect(opened).toBe(false);
		});

	for (const raw of [
		"HTTP/1.1 200 OK\n\n",
		"HTTP/1.1 200 OK\r\n Content-Length: 1\r\n\r\n",
		"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\na",
		"HTTP/1.1 200 OK\r\nContent-Length: 1, 1\r\n\r\na",
		"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
		"HTTP/1.1 200 OK\r\nContent-Length: 9007199254740992\r\n\r\n",
		"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
		"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
		"HTTP/1.1 101 Switching Protocols\r\n\r\n",
		"HTTP/1.1 204 No Content\r\nContent-Length: 1\r\n\r\na",
		"HTTP/1.1 100 Continue\r\nContent-Length: 1\r\n\r\na",
		"HTTP/1.1 200 OK\r\nX: a\u0000b\r\n\r\n",
		`HTTP/1.1 200 OK\r\nX: ${"a".repeat(TUNNEL_HTTP_HEADER_LIMIT)}\r\n\r\n`,
	])
		test(`rejects malformed response headers: ${raw.slice(0, 90)}`, async () => {
			await expect(requestResponse(raw)).rejects.toThrow(
				"Invalid service HTTP response",
			);
		});

	for (const raw of [
		"Content-Length: 5\r\n\r\na",
		"Transfer-Encoding: chunked\r\n\r\nz\r\n",
		"Transfer-Encoding: chunked\r\n\r\n2\r\na",
		"Transfer-Encoding: chunked\r\n\r\n1\r\naX\r\n0\r\n\r\n",
		"Transfer-Encoding: chunked\r\n\r\n1;bad=\r\na\r\n0\r\n\r\n",
		"Transfer-Encoding: chunked\r\n\r\n0\r\nContent-Length: 0\r\n\r\n",
		`Transfer-Encoding: chunked\r\n\r\n1;${"a".repeat(8192)}\r\na\r\n0\r\n\r\n`,
	])
		test(`resets malformed response body: ${raw.slice(0, 80)}`, async () => {
			const { response, peer } = await requestResponse(
				`HTTP/1.1 200 OK\r\n${raw}`,
			);
			await expect(text(response)).rejects.toThrow(
				"Invalid service HTTP response",
			);
			await settle();
			expect(peer.sent.some((frame) => frame.kind === TunnelKind.Reset)).toBe(
				true,
			);
		});

	test("cancels a pending connection and resets a late successful open", async () => {
		const controller = new AbortController();
		let resolve!: (stream: DeviceServiceStream) => void;
		let reset = false;
		const pending = tunnelHttpRequest(
			() =>
				new Promise((yes) => {
					resolve = yes;
				}),
			{ method: "GET", path: "/", signal: controller.signal },
		);
		controller.abort();
		await expect(pending).rejects.toThrow("cancelled");
		resolve({
			reset: () => {
				reset = true;
			},
		} as unknown as DeviceServiceStream);
		await settle();
		expect(reset).toBe(true);
	});

	test("aborts headers while a request write is pending", async () => {
		const controller = new AbortController();
		let reset = false;
		const stream = {
			read: () => new Promise(() => {}),
			write: () => new Promise(() => {}),
			reset: () => {
				reset = true;
			},
		} as unknown as DeviceServiceStream;
		const pending = tunnelHttpRequest(async () => stream, {
			method: "POST",
			path: "/",
			body: "body",
			signal: controller.signal,
		});
		await settle();
		controller.abort();
		await expect(pending).rejects.toThrow("cancelled");
		expect(reset).toBe(true);
	});

	test("bounds connection waits and body inactivity without a total SSE deadline", async () => {
		const original = globalThis.setTimeout;
		const timers: { callback: () => void; duration?: number }[] = [];
		globalThis.setTimeout = ((callback: () => void, duration?: number) => {
			timers.push({ callback, duration });
			return 1;
		}) as typeof setTimeout;
		try {
			const pending = tunnelHttpRequest(() => new Promise(() => {}), {
				method: "GET",
				path: "/",
			});
			expect(timers.at(-1)?.duration).toBe(TUNNEL_HTTP_OPEN_TIMEOUT);
			timers.at(-1)?.callback();
			await expect(pending).rejects.toThrow("timed out");
			let reads = 0;
			let reset = false;
			const stream = {
				read: () =>
					++reads === 1
						? Promise.resolve(
								encoder.encode(
									"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n",
								),
							)
						: new Promise(() => {}),
				write: async () => {},
				reset: () => {
					reset = true;
				},
			} as unknown as DeviceServiceStream;
			const response = await tunnelHttpRequest(async () => stream, {
				method: "GET",
				path: "/",
			});
			const body = response.body[Symbol.asyncIterator]().next();
			expect(timers.at(-1)?.duration).toBe(TUNNEL_HTTP_READ_TIMEOUT);
			timers.at(-1)?.callback();
			await expect(body).rejects.toThrow("stalled");
			expect(reset).toBe(true);
		} finally {
			globalThis.setTimeout = original;
		}
	});
});

test("real local HTTP server works through the byte mux", async () => {
	const server = createServer(async (request, response) => {
		let body = "";
		for await (const chunk of request) body += String(chunk);
		expect(request.url).toBe("/api?local=1");
		expect(body).toBe("payload");
		response.writeHead(200, { "content-type": "text/plain" });
		response.write("first ");
		response.end("second");
	});
	await new Promise<void>((resolve, reject) => {
		server.once("error", reject);
		server.listen(0, "127.0.0.1", resolve);
	});
	const address = server.address();
	if (!address || typeof address === "string")
		throw new Error("HTTP fixture did not bind.");
	const socket = connect({ host: "127.0.0.1", port: address.port });
	try {
		await new Promise<void>((resolve, reject) => {
			socket.once("connect", resolve);
			socket.once("error", reject);
		});
		const peer = new Peer();
		const tunnel = peer.connect();
		peer.onData = (bytes) => {
			socket.write(bytes);
		};
		peer.onReset = () => {
			socket.destroy();
		};
		socket.on("data", (bytes: Buffer) => {
			for (let offset = 0; offset < bytes.length; offset += TUNNEL_MAX_DATA)
				peer.emit(
					TunnelKind.Data,
					bytes.slice(offset, offset + TUNNEL_MAX_DATA),
				);
		});
		socket.on("end", () => peer.emit(TunnelKind.Fin));
		const response = await tunnelHttpRequest(
			(signal) => tunnel.open("p", "hosting", { signal }),
			{ method: "POST", path: "/api?local=1", body: "payload" },
		);
		expect(await text(response)).toBe("first second");
	} finally {
		socket.destroy();
		await new Promise<void>((resolve) => server.close(() => resolve()));
	}
});
