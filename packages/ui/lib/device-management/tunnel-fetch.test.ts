import { describe, expect, test } from "bun:test";
import type { DeviceServiceStream } from "./tunnel";
import { createTunnelFetch } from "./tunnel-fetch";
import { TUNNEL_HTTP_BODY_LIMIT } from "./tunnel-http";

const encode = (text: string) => new TextEncoder().encode(text);
function fixture(chunks: string[], options: { signal?: AbortSignal } = {}) {
	let reads = 0;
	let resets = 0;
	let opens = 0;
	const writes: Uint8Array[] = [];
	let fail: ((error: Error) => void) | undefined;
	const stream = {
		finished: false,
		async read() {
			reads++;
			return chunks.length
				? encode(chunks.shift() ?? "")
				: new Promise<Uint8Array | null>((_, reject) => {
						fail = reject;
					});
		},
		async write(bytes: Uint8Array) {
			writes.push(bytes.slice());
		},
		reset() {
			resets++;
			fail?.(new Error("reset"));
		},
	} as unknown as DeviceServiceStream;
	const fetcher = createTunnelFetch({
		open: async () => {
			opens++;
			return stream;
		},
		signal: options.signal,
	});
	return {
		fetcher,
		get reads() {
			return reads;
		},
		get resets() {
			return resets;
		},
		get opens() {
			return opens;
		},
		get request() {
			return writes.map((bytes) => new TextDecoder().decode(bytes)).join("");
		},
	};
}

describe("deployed service fetch", () => {
	test("creates consumer-driven response streams and releases cancelled bodies", async () => {
		const peer = fixture([
			"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\n",
			"abc",
			"def",
		]);
		const response = await peer.fetcher("/chat/event");
		expect(peer.reads).toBe(1);
		await Promise.resolve();
		expect(peer.reads).toBe(1);
		if (!response.body) throw new Error("Expected a response body");
		const reader = response.body.getReader();
		expect(new TextDecoder().decode((await reader.read()).value)).toBe("abc");
		expect(peer.reads).toBe(2);
		await reader.cancel();
		expect(peer.resets).toBe(1);
		expect(peer.request).toContain("Host: localhost\r\n");
	});

	test("supports bounded File uploads and binary responses", async () => {
		const peer = fixture([
			"HTTP/1.1 201 Created\r\nContent-Length: 3\r\n\r\n",
			"yes",
		]);
		const response = await peer.fetcher("/run/event", {
			method: "POST",
			body: new File(["héllo"], "input.txt", { type: "text/plain" }),
		});
		expect(await response.text()).toBe("yes");
		expect(peer.request).toContain("Content-Length: 6\r\n");
		expect(peer.request).toContain("content-type: text/plain");
		expect(peer.request.endsWith("héllo")).toBe(true);
	});

	test("a lifetime abort errors a body even when no read is pending", async () => {
		const lifetime = new AbortController();
		const peer = fixture(["HTTP/1.1 200 OK\r\n\r\n"], {
			signal: lifetime.signal,
		});
		const response = await peer.fetcher("/run/event");
		lifetime.abort();
		await expect(response.text()).rejects.toThrow(/cancelled/);
		expect(peer.resets).toBe(1);
	});

	test("a request abort resets its pending read", async () => {
		const controller = new AbortController();
		const peer = fixture(["HTTP/1.1 200 OK\r\n\r\n"]);
		const response = await peer.fetcher("/run/event", {
			signal: controller.signal,
		});
		const reading = response.text();
		controller.abort();
		await expect(reading).rejects.toThrow(/cancelled/);
		expect(peer.resets).toBe(1);
	});

	test("validates paths and cookie overrides before opening a service", async () => {
		const peer = fixture([]);
		for (const path of [
			"https://attacker.test/",
			"//attacker.test",
			"/x/../services",
			"/x/%2e%2e/services",
			"/x/%252e%252e/services",
			"/x%2fy",
			"/x\\y",
			"/x#fragment",
			"/x\r\ny",
		])
			await expect(peer.fetcher(path)).rejects.toThrow();
		await expect(
			peer.fetcher("/services", { credentials: "include" }),
		).rejects.toThrow(/cookies/);
		await expect(
			peer.fetcher("/services", { headers: { Cookie: "secret" } }),
		).rejects.toThrow(/controlled/);
		await expect(
			peer.fetcher("/services", { redirect: "follow" }),
		).rejects.toThrow(/redirect/);
		expect(peer.opens).toBe(0);
	});

	test("preserves redirect responses without cookies, with explicit redirect error support", async () => {
		const raw =
			"HTTP/1.1 302 Found\r\nLocation: https://attacker.test\r\nSet-Cookie: secret=x\r\nContent-Length: 0\r\n\r\n";
		const peer = fixture([raw]);
		const response = await peer.fetcher("/services");
		expect(response.status).toBe(302);
		expect(response.headers.get("set-cookie")).toBeNull();
		await response.body?.cancel();
		const refused = fixture([raw]);
		await expect(
			refused.fetcher("/services", { redirect: "error" }),
		).rejects.toThrow(/redirect/);
		expect(refused.opens).toBe(1);
		expect(refused.resets).toBe(1);
	});

	test("rejects oversized bodies before connecting", async () => {
		const peer = fixture([]);
		await expect(
			peer.fetcher("/run/event", {
				method: "POST",
				body: new Uint8Array(TUNNEL_HTTP_BODY_LIMIT + 1),
			}),
		).rejects.toThrow(/10 MiB/);
		await expect(
			peer.fetcher("/run/event", {
				method: "POST",
				body: new Blob([new Uint8Array(TUNNEL_HTTP_BODY_LIMIT + 1)]),
			}),
		).rejects.toThrow(/10 MiB/);
		expect(peer.opens).toBe(0);
	});

	test("accepts request bodies above the former 8 MiB limit", async () => {
		const peer = fixture(["HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"]);
		const response = await peer.fetcher("/run/event", {
			method: "POST",
			body: new Uint8Array(9 * 1024 * 1024),
		});
		await response.body?.cancel();
		expect(peer.opens).toBe(1);
		expect(peer.request).toContain(`Content-Length: ${9 * 1024 * 1024}\r\n`);
	});

	test("HEAD responses have no body and release their connection", async () => {
		const peer = fixture(["HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n"]);
		const response = await peer.fetcher("/ui/assets/1234567890123456", {
			method: "HEAD",
		});
		expect(response.body).toBeNull();
		expect(peer.resets).toBe(1);
	});
});
