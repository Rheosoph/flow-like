import { describe, expect, it } from "bun:test";
import {
	type QueryStorageBackend,
	createBoundedStorage,
	createSmartQueryPersister,
	shouldPersistQuery,
} from "./query-persister";

function memoryBackend(): QueryStorageBackend & { map: Map<string, string> } {
	const map = new Map<string, string>();
	return {
		map,
		get: async (key) => map.get(key),
		set: async (key, value) => {
			map.set(key, value);
		},
		del: async (key) => {
			map.delete(key);
		},
		entries: async () => [...map.entries()],
	};
}

describe("shouldPersistQuery", () => {
	it("persists ordinary backend queries by default", () => {
		expect(shouldPersistQuery({ queryKey: ["getApps"] })).toBe(true);
		expect(shouldPersistQuery({ queryKey: ["getProfile", "user-1"] })).toBe(
			true,
		);
		expect(
			shouldPersistQuery({ queryKey: ["searchApps", "query", null] }),
		).toBe(true);
	});

	it("denies the measured offenders and admin data", () => {
		expect(shouldPersistQuery({ queryKey: ["ai-act", "inventory", "a"] })).toBe(
			false,
		);
		expect(shouldPersistQuery({ queryKey: ["admin", "usage"] })).toBe(false);
		expect(shouldPersistQuery({ queryKey: ["getRoles", "app"] })).toBe(false);
		expect(shouldPersistQuery({ queryKey: ["getRoutes", "app"] })).toBe(false);
	});

	it("denies keys without an identifiable string head", () => {
		expect(shouldPersistQuery({ queryKey: [] })).toBe(false);
		expect(shouldPersistQuery({ queryKey: [42, "x"] })).toBe(false);
		expect(shouldPersistQuery({ queryKey: [""] })).toBe(false);
	});

	it("meta.persist overrides both directions", () => {
		expect(
			shouldPersistQuery({
				queryKey: ["getApps"],
				meta: { persist: false },
			}),
		).toBe(false);
		expect(
			shouldPersistQuery({
				queryKey: ["ai-act", "inventory"],
				meta: { persist: true },
			}),
		).toBe(true);
	});
});

describe("createBoundedStorage", () => {
	it("stores and retrieves entries under the cap", async () => {
		const backend = memoryBackend();
		const storage = createBoundedStorage(backend, 100);
		await storage.setItem("a", "small");
		expect(await storage.getItem("a")).toBe("small");
		expect(await storage.entries()).toEqual([["a", "small"]]);
		await storage.removeItem("a");
		expect(await storage.getItem("a")).toBeUndefined();
	});

	it("refuses oversized entries and drops the stale previous version", async () => {
		const backend = memoryBackend();
		const storage = createBoundedStorage(backend, 100);
		await storage.setItem("q", "previously-small");
		await storage.setItem("q", "x".repeat(101));
		expect(await storage.getItem("q")).toBeUndefined();
		expect(backend.map.size).toBe(0);
	});

	it("swallows backend failures so persistence never breaks queries", async () => {
		const broken: QueryStorageBackend = {
			get: async () => {
				throw new Error("idb unavailable");
			},
			set: async () => {
				throw new Error("idb unavailable");
			},
			del: async () => {
				throw new Error("idb unavailable");
			},
			entries: async () => {
				throw new Error("idb unavailable");
			},
		};
		const storage = createBoundedStorage(broken, 100);
		expect(await storage.getItem("a")).toBeUndefined();
		await storage.setItem("a", "value");
		await storage.removeItem("a");
		expect(await storage.entries()).toEqual([]);

		// End to end: a query on broken storage still resolves via its queryFn
		const persister = createSmartQueryPersister({ backend: broken });
		const result = await persister.persisterFn(
			async () => ({ ok: true }),
			{} as never,
			{
				queryKey: ["getApps"],
				queryHash: '["getApps"]',
				state: { data: undefined, dataUpdatedAt: 0, errorUpdatedAt: 0 },
				setState: () => {},
				isStale: () => false,
				fetch: () => {},
			} as never,
		);
		expect(result).toEqual({ ok: true });
	});
});

describe("createSmartQueryPersister end to end", () => {
	interface FakeQuery {
		queryKey: readonly unknown[];
		queryHash: string;
		meta?: Record<string, unknown>;
		state: {
			data: unknown;
			dataUpdatedAt: number;
			errorUpdatedAt: number;
		};
		setState: (s: Record<string, unknown>) => void;
		isStale: () => boolean;
		fetch: () => void;
	}

	function fakeQuery(
		queryKey: readonly unknown[],
		data: unknown = undefined,
	): FakeQuery {
		return {
			queryKey,
			queryHash: JSON.stringify(queryKey),
			state: { data, dataUpdatedAt: Date.now(), errorUpdatedAt: 0 },
			setState: () => {},
			isStale: () => false,
			fetch: () => {},
		};
	}

	const ctx = {} as never;

	it("persists allowed queries and restores them on next fetch", async () => {
		const backend = memoryBackend();
		const persister = createSmartQueryPersister({ backend });

		const query = fakeQuery(["getApps"]);
		const result = await persister.persisterFn(
			async () => ({ apps: [1, 2, 3] }),
			ctx,
			query as never,
		);
		expect(result).toEqual({ apps: [1, 2, 3] });
		query.state.data = result;
		// persistQuery is scheduled via notifyManager — give it a tick
		await new Promise((r) => setTimeout(r, 10));
		expect(backend.map.size).toBe(1);

		// A fresh query with empty cache restores from storage without fetching
		const restored = await persister.persisterFn(
			async () => {
				throw new Error("should not fetch");
			},
			ctx,
			fakeQuery(["getApps"]) as never,
		);
		expect(restored).toEqual({ apps: [1, 2, 3] });
	});

	it("never touches storage for denied queries", async () => {
		const backend = memoryBackend();
		const persister = createSmartQueryPersister({ backend });

		const result = await persister.persisterFn(
			async () => ({ huge: true }),
			ctx,
			fakeQuery(["ai-act", "inventory"]) as never,
		);
		expect(result).toEqual({ huge: true });
		await new Promise((r) => setTimeout(r, 10));
		expect(backend.map.size).toBe(0);
	});

	it("drops entries that grew past the size cap", async () => {
		const backend = memoryBackend();
		const persister = createSmartQueryPersister({
			backend,
			maxEntryBytes: 200,
		});

		const query = fakeQuery(["getApps"]);
		const big = { blob: "x".repeat(500) };
		query.state.data = big;
		const result = await persister.persisterFn(
			async () => big,
			ctx,
			query as never,
		);
		expect(result).toEqual(big);
		await new Promise((r) => setTimeout(r, 10));
		expect(backend.map.size).toBe(0);
	});

	it("garbage-collects expired entries", async () => {
		const backend = memoryBackend();
		const persister = createSmartQueryPersister({ backend, maxAge: 50 });

		const query = fakeQuery(["getApps"]);
		query.state.data = { apps: [] };
		await persister.persisterFn(
			async () => ({ apps: [] }),
			ctx,
			query as never,
		);
		await new Promise((r) => setTimeout(r, 10));
		expect(backend.map.size).toBe(1);

		await new Promise((r) => setTimeout(r, 60));
		await persister.persisterGc();
		expect(backend.map.size).toBe(0);
	});
});
