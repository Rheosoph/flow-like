import { beforeEach, describe, expect, it } from "vitest";
import { MockHostBridge, getHost, setHost } from "../src/host";
import type { FlowPath } from "../src/host";

describe("MockHostBridge", () => {
	let host: MockHostBridge;

	beforeEach(() => {
		host = new MockHostBridge();
	});

	describe("logging", () => {
		it("records log calls", () => {
			host.log(0, "debug msg");
			host.log(1, "info msg");
			expect(host.logs).toHaveLength(2);
			expect(host.logs[0]).toEqual([0, "debug msg"]);
			expect(host.logs[1]).toEqual([1, "info msg"]);
		});
	});

	describe("streaming", () => {
		it("records stream events", () => {
			host.stream("progress", '{"pct":50}');
			expect(host.streams).toHaveLength(1);
			expect(host.streams[0]).toEqual(["progress", '{"pct":50}']);
		});

		it("records streamText as text type", () => {
			host.streamText("hello world");
			expect(host.streams[0]).toEqual(["text", "hello world"]);
		});
	});

	describe("variables", () => {
		it("sets and gets variables", () => {
			expect(host.getVariable("x")).toBeNull();
			host.setVariable("x", { value: 42 });
			expect(host.getVariable("x")).toEqual({ value: 42 });
		});

		it("deletes variables", () => {
			host.setVariable("x", 1);
			host.deleteVariable("x");
			expect(host.getVariable("x")).toBeNull();
		});

		it("checks variable existence", () => {
			expect(host.hasVariable("x")).toBe(false);
			host.setVariable("x", 1);
			expect(host.hasVariable("x")).toBe(true);
		});
	});

	describe("cache", () => {
		it("sets and gets cache values", () => {
			expect(host.cacheGet("k")).toBeNull();
			host.cacheSet("k", [1, 2, 3]);
			expect(host.cacheGet("k")).toEqual([1, 2, 3]);
		});

		it("deletes cache entries", () => {
			host.cacheSet("k", 1);
			host.cacheDelete("k");
			expect(host.cacheGet("k")).toBeNull();
		});

		it("checks cache existence", () => {
			expect(host.cacheHas("k")).toBe(false);
			host.cacheSet("k", 1);
			expect(host.cacheHas("k")).toBe(true);
		});
	});

	describe("time and random", () => {
		it("returns deterministic time", () => {
			expect(host.timeNow()).toBe(0);
		});

		it("returns deterministic random", () => {
			expect(host.random()).toBe(42);
		});
	});

	describe("storage", () => {
		it("returns storage directory paths", () => {
			const dir = host.storageDir(false) as FlowPath;
			expect(dir.path).toBe("storage");
			expect(dir.store_ref).toBe("mock_store");

			const nodeDir = host.storageDir(true) as FlowPath;
			expect(nodeDir.path).toBe("storage/node");
		});

		it("returns upload directory", () => {
			const dir = host.uploadDir() as FlowPath;
			expect(dir.path).toBe("upload");
		});

		it("reads and writes storage", () => {
			const path: FlowPath = {
				path: "test/file.bin",
				store_ref: "s",
				cache_store_ref: null,
			};
			const data = new Uint8Array([1, 2, 3]);

			expect(host.storageRead(path)).toBeNull();
			expect(host.storageWrite(path, data)).toBe(true);
			expect(host.storageRead(path)).toEqual(data);
		});

		it("lists storage entries", () => {
			const base: FlowPath = {
				path: "dir/",
				store_ref: "s",
				cache_store_ref: null,
			};
			host.storageWrite(
				{ path: "dir/a.txt", store_ref: "s", cache_store_ref: null },
				new Uint8Array([1]),
			);
			host.storageWrite(
				{ path: "dir/b.txt", store_ref: "s", cache_store_ref: null },
				new Uint8Array([2]),
			);
			host.storageWrite(
				{ path: "other/c.txt", store_ref: "s", cache_store_ref: null },
				new Uint8Array([3]),
			);

			const list = host.storageList(base);
			expect(list).toHaveLength(2);
		});
	});

	describe("embeddings", () => {
		it("returns embeddings for texts", () => {
			const result = host.embedText({}, ["hello", "world"]);
			expect(result).toHaveLength(2);
			expect(result[0]).toEqual([0.1, 0.2, 0.3]);
		});
	});

	describe("OAuth", () => {
		it("returns null when no token", () => {
			expect(host.getOAuthToken("github")).toBeNull();
			expect(host.hasOAuthToken("github")).toBe(false);
		});

		it("returns token when set", () => {
			host.oauthTokens.github = { access_token: "abc" };
			expect(host.hasOAuthToken("github")).toBe(true);
			expect(host.getOAuthToken("github")).toEqual({ access_token: "abc" });
		});
	});
});

describe("setHost / getHost", () => {
	it("sets and retrieves custom host", () => {
		const mock = new MockHostBridge();
		setHost(mock);
		expect(getHost()).toBe(mock);
	});

	it("default host has no-op methods", () => {
		// Reset by getting current host - just verify it doesn't throw
		const host = getHost();
		expect(() => host.log(0, "test")).not.toThrow();
		expect(host.getVariable("x")).toBeNull();
		expect(host.timeNow()).toBeDefined();
	});
});
