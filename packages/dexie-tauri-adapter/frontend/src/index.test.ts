import { describe, it, expect, mock, beforeEach } from "bun:test";

// Mock @tauri-apps/api/core before importing our module
const invokeResults = new Map<string, unknown>();
mock.module("@tauri-apps/api/core", () => ({
	invoke: async (cmd: string, args?: Record<string, unknown>) => {
		const handler = invokeResults.get(cmd);
		if (typeof handler === "function") return handler(args);
		if (handler !== undefined) return handler;
		throw new Error(`Unhandled invoke: ${cmd}`);
	},
}));

// Mock dexie types (we only need the type guard behavior)
mock.module("dexie", () => ({}));

const {
	dexieTauriBlobOffload,
	configureBlobOffload,
} = await import("./index");

const PLUGIN_PREFIX = "plugin:flow-like-dexie-blob-offload|";
const BLOB_MARKER = "__fl_blob__";

// Helpers
let storedBlobs: Map<string, { data: number[]; mac: string }>;
let nextMac: number;

function setupMockBackend() {
	storedBlobs = new Map();
	nextMac = 1;

	invokeResults.set(
		`${PLUGIN_PREFIX}blob_store`,
		(args: { data: number[] }) => {
			const hash = `hash_${storedBlobs.size}`;
			const mac = `mac_${nextMac++}`;
			storedBlobs.set(hash, { data: args.data, mac });
			return { hash, mac };
		},
	);

	invokeResults.set(
		`${PLUGIN_PREFIX}blob_store_batch`,
		(args: { entries: Array<{ key: string; data: number[] }> }) => {
			return args.entries.map((entry) => {
				const hash = `hash_${storedBlobs.size}`;
				const mac = `mac_${nextMac++}`;
				storedBlobs.set(hash, { data: entry.data, mac });
				return { key: entry.key, blob_ref: { hash, mac } };
			});
		},
	);

	invokeResults.set(
		`${PLUGIN_PREFIX}blob_get`,
		(args: { hash: string; mac: string }) => {
			const blob = storedBlobs.get(args.hash);
			if (!blob || blob.mac !== args.mac)
				throw new Error("Invalid blob reference");
			return blob.data;
		},
	);

	invokeResults.set(
		`${PLUGIN_PREFIX}blob_get_batch`,
		(args: { refs: Array<{ key: string; blob_ref: { hash: string; mac: string } }> }) => {
			return args.refs.map((entry) => {
				const blob = storedBlobs.get(entry.blob_ref.hash);
				if (!blob || blob.mac !== entry.blob_ref.mac)
					throw new Error("Invalid blob reference");
				return { key: entry.key, data: blob.data };
			});
		},
	);

	invokeResults.set(
		`${PLUGIN_PREFIX}blob_configure`,
		(_args: { basePath: string }) => undefined,
	);
}

// Create a minimal DBCore mock
function createMockDBCore() {
	const rows = new Map<number, unknown>();
	let nextId = 1;

	const table = {
		name: "test",
		schema: {
			name: "test",
			primKey: { keyPath: "id", name: "id" },
			indexes: [],
			mappedClass: null,
		},
		mutate: async (req: {
			type: string;
			values?: unknown[];
			keys?: unknown[];
		}) => {
			if (req.type === "add" || req.type === "put") {
				const addedKeys: number[] = [];
				for (const val of req.values || []) {
					const id = nextId++;
					const stored = { ...(val as Record<string, unknown>), id };
					rows.set(id, stored);
					addedKeys.push(id);
				}
				return {
					numFailures: 0,
					failures: {},
					lastResult: addedKeys[addedKeys.length - 1],
					results: addedKeys,
				};
			}
			if (req.type === "delete") {
				for (const k of req.keys || []) rows.delete(k as number);
				return { numFailures: 0, failures: {}, results: [] };
			}
			return { numFailures: 0, failures: {}, results: [] };
		},
		get: async (req: { key: unknown; trans: unknown }) => rows.get(req.key as number) ?? undefined,
		getMany: async (req: { keys: unknown[]; trans: unknown }) =>
			req.keys.map((k) => rows.get(k as number) ?? undefined),
		query: async (_req: unknown) => ({
			result: Array.from(rows.values()),
		}),
		openCursor: async () => null,
		count: async () => rows.size,
	};

	return {
		stack: "dbcore" as const,
		table: (_name: string) => table,
		cmp: (a: any, b: any) => (a === b ? 0 : a < b ? -1 : 1),
		MIN_KEY: -Infinity,
		MAX_KEY: [[]],
		schema: { name: "testdb", tables: [] },
		_rows: rows,
	};
}

describe("dexieTauriBlobOffload", () => {
	beforeEach(setupMockBackend);

	it("returns a valid middleware descriptor", () => {
		const mw = dexieTauriBlobOffload(100);
		expect(mw.stack).toBe("dbcore");
		expect(mw.name).toBe("flow-like-dexie-tauri-blob-offload");
		expect(typeof mw.create).toBe("function");
	});

	it("passes through small values unchanged", async () => {
		const mw = dexieTauriBlobOffload(100);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		const small = { name: "tiny", data: "short" };
		await table.mutate({ type: "add", values: [small] } as any);

		const stored = core._rows.get(1) as Record<string, unknown>;
		expect(stored.name).toBe("tiny");
		expect(stored.data).toBe("short");
	});

	it("offloads large strings and rehydrates on get", async () => {
		const mw = dexieTauriBlobOffload(10);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		const largeStr = "A".repeat(50);
		await table.mutate({
			type: "add",
			values: [{ content: largeStr }],
		} as any);

		// The stored value should have a blob marker, not the raw string
		const raw = core._rows.get(1) as Record<string, unknown>;
		expect(raw.content).toHaveProperty(BLOB_MARKER);

		// Reading via the middleware should rehydrate
		const result = (await table.get({ key: 1 } as any)) as Record<string, unknown>;
		expect(result.content).toBe(largeStr);
	});

	it("offloads large number arrays and rehydrates", async () => {
		const mw = dexieTauriBlobOffload(5);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		// Use bytes 0x80-0x93 which are invalid standalone UTF-8 — ensures
		// tryDecodeUtf8 falls back to returning the raw number array.
		const largeArr = Array.from({ length: 20 }, (_, i) => 0x80 + i);
		await table.mutate({
			type: "add",
			values: [{ data: largeArr }],
		} as any);

		const raw = core._rows.get(1) as Record<string, unknown>;
		expect(raw.data).toHaveProperty(BLOB_MARKER);

		const result = (await table.get({ key: 1 } as any)) as Record<string, unknown>;
		expect(result.data).toEqual(largeArr);
	});

	it("handles nested large values", async () => {
		const mw = dexieTauriBlobOffload(10);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		const obj = {
			meta: { title: "short" },
			payload: { body: "X".repeat(50), notes: "Y".repeat(50) },
		};
		await table.mutate({ type: "add", values: [obj] } as any);

		const result = (await table.get({ key: 1 } as any)) as any;
		expect(result.meta.title).toBe("short");
		expect(result.payload.body).toBe("X".repeat(50));
		expect(result.payload.notes).toBe("Y".repeat(50));
	});

	it("rehydrates via getMany", async () => {
		const mw = dexieTauriBlobOffload(5);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		await table.mutate({
			type: "add",
			values: [
				{ name: "a", data: "Z".repeat(20) },
				{ name: "b", data: "W".repeat(20) },
			],
		} as any);

		const results = (await table.getMany({ keys: [1, 2] } as any)) as any[];
		expect(results[0].data).toBe("Z".repeat(20));
		expect(results[1].data).toBe("W".repeat(20));
	});

	it("rehydrates via query", async () => {
		const mw = dexieTauriBlobOffload(5);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		await table.mutate({
			type: "add",
			values: [{ content: "Q".repeat(30) }],
		} as any);

		const qr = await table.query({} as any);
		expect((qr.result[0] as any).content).toBe("Q".repeat(30));
	});

	it("leaves null/undefined values untouched", async () => {
		const mw = dexieTauriBlobOffload(5);
		const core = createMockDBCore();
		const wrapped = mw.create(core as any);
		const table = wrapped.table("test");

		await table.mutate({
			type: "add",
			values: [{ a: null, b: undefined, c: 42 }],
		} as any);

		const result = (await table.get({ key: 1 } as any)) as any;
		expect(result.a).toBeNull();
		expect(result.c).toBe(42);
	});
});

describe("configureBlobOffload", () => {
	beforeEach(setupMockBackend);

	it("calls the configure command", async () => {
		let calledWith: string | undefined;
		invokeResults.set(
			`${PLUGIN_PREFIX}blob_configure`,
			(args: { basePath: string }) => {
				calledWith = args.basePath;
				return undefined;
			},
		);

		await configureBlobOffload("/custom/path");
		expect(calledWith).toBe("/custom/path");
	});
});
