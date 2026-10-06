import { afterEach, describe, expect, spyOn, test } from "bun:test";
import type { FormMemoryTables } from "../../../../db/form-memory-db";
import {
	type CreateFormMemoryStore,
	FIELD_KEY_SEPARATOR,
	FORM_LIMITS,
	type FormMemoryKey,
	type FormPrefs,
	type Preset,
	type StoredInput,
	type StoredRunRecord,
} from "../contracts";
import {
	createFormMemoryStore,
	createMapTables,
	createMemoryStoreOn,
	fitPreset,
	prefsOf,
	prunePrefs,
	runOf,
	withoutSecrets,
} from "./form-memory";

const _contract: CreateFormMemoryStore = createFormMemoryStore;

const KEY: FormMemoryKey = {
	scope: "profile:p1",
	appId: "app-1",
	eventId: "event-1",
};
const NOW = Date.UTC(2026, 9, 5);
const DAY = 24 * 60 * 60 * 1000;

function record(
	n: number,
	inputs: Record<string, StoredInput> = {},
	extra: Partial<StoredRunRecord> = {},
) {
	const made: StoredRunRecord = {
		version: 2,
		id: `run-${n}`,
		...KEY,
		n,
		createdAt: n * 1000,
		startedAt: n * 1000,
		endedAt: n * 1000 + 500,
		status: "done",
		outcome: { kind: "succeeded" },
		summary: {
			firstLine: null,
			stepCount: 0,
			stepReached: null,
			fileCount: 0,
			hasAnswer: false,
			hasResult: false,
		},
		failedAt: null,
		presetName: null,
		inputs,
		perRun: [],
		replaced: [],
		...extra,
	};
	return made;
}

function prefs(extra: Partial<FormPrefs> = {}) {
	const made: FormPrefs = {
		version: 2,
		perRun: [],
		auto: [],
		introduced: false,
		offerAnswered: false,
		noSave: [],
		forgotten: {},
		nextRunNumber: 1,
		fieldSeenAt: {},
		...extra,
	};
	return made;
}

function preset(id: string, extra: Partial<Preset> = {}) {
	const made: Preset = {
		id,
		name: id,
		digit: 1,
		sets: { vendor_name: "Nordwind Logistik GmbH" },
		kinds: { vendor_name: "text" },
		openDefault: false,
		createdAt: 1000,
		updatedAt: 1000,
		lastUsedAt: null,
		...extra,
	};
	return made;
}

function sessionStore() {
	const map = new Map<string, unknown>();
	const tables = createMapTables(map);
	const store = createMemoryStoreOn(tables, "session", () => NOW);
	return { map, tables, store };
}

let warn: ReturnType<typeof spyOn> | null = null;
afterEach(() => {
	warn?.mockRestore();
	warn = null;
});
const quietWarnings = () => {
	warn = spyOn(console, "warn").mockImplementation(() => undefined);
	return warn;
};

describe("runs", () => {
	test("an empty form loads as nothing saved", async () => {
		const { store } = sessionStore();
		expect(store.mode).toBe("session");
		expect(await store.load(KEY)).toEqual({
			prefs: null,
			presets: [],
			runs: [],
		});
	});

	test("runs load newest first, rows hold JSON strings", async () => {
		const { store, tables } = sessionStore();
		for (const n of [2, 3, 1]) await store.putRun(record(n));
		const loaded = await store.load(KEY);
		expect(loaded.runs.map((run) => run.n)).toEqual([3, 2, 1]);
		const rows = await tables.read("runs", KEY);
		expect(rows.every((row) => typeof row.json === "string")).toBe(true);
	});

	test("writing a run again replaces its row", async () => {
		const { store } = sessionStore();
		await store.putRun(record(1, {}, { status: "running" }));
		await store.putRun(record(1, {}, { status: "done" }));
		const loaded = await store.load(KEY);
		expect(loaded.runs).toHaveLength(1);
		expect(loaded.runs[0].status).toBe("done");
	});

	test(`only the newest ${FORM_LIMITS.historyPerForm} runs of a form are kept`, async () => {
		const { store, tables } = sessionStore();
		const total = FORM_LIMITS.historyPerForm + 5;
		for (let n = 1; n <= total; n++) await store.putRun(record(n));
		expect(await tables.runIdsOldestFirst(KEY)).toHaveLength(
			FORM_LIMITS.historyPerForm,
		);
		const loaded = await store.load(KEY);
		expect(loaded.runs[0].n).toBe(total);
		expect(loaded.runs.at(-1)?.n).toBe(6);
	});

	test("deleteRun removes one run", async () => {
		const { store } = sessionStore();
		await store.putRun(record(1));
		await store.putRun(record(2));
		await store.deleteRun(KEY, "run-1");
		expect((await store.load(KEY)).runs.map((run) => run.id)).toEqual([
			"run-2",
		]);
	});

	test("profiles, apps and events never see each other's runs", async () => {
		const { store } = sessionStore();
		const other = { ...KEY, scope: "profile:p2" };
		await store.putRun(record(1));
		await store.putRun({ ...record(2), ...other });
		await store.putRun({ ...record(3), appId: "app-2" });
		expect((await store.load(KEY)).runs.map((run) => run.n)).toEqual([1]);
		expect((await store.load(other)).runs.map((run) => run.n)).toEqual([2]);
	});
});

describe("hideField", () => {
	test('"Don\'t save" rewrites that input in every saved run of the form', async () => {
		const { store } = sessionStore();
		await store.putRun(
			record(1, { vendor_name: "Nordwind Logistik GmbH", max_pages: 20 }),
		);
		await store.putRun(record(2, { vendor_name: "Alpenfracht AG" }));
		await store.putRun(record(3, { max_pages: 40 }));
		await store.hideField(KEY, "vendor_name");
		const runs = (await store.load(KEY)).runs;
		expect(runs.map((run) => run.inputs)).toEqual([
			{ max_pages: 40 },
			{ vendor_name: { $hidden: true } },
			{ vendor_name: { $hidden: true }, max_pages: 20 },
		]);
	});

	test("an object property is hidden inside its object", async () => {
		const { store } = sessionStore();
		await store.putRun(
			record(1, { address: { city: "Berlin", zip: "10115" } }),
		);
		await store.hideField(KEY, `address${FIELD_KEY_SEPARATOR}city`);
		const [run] = (await store.load(KEY)).runs;
		expect(run.inputs).toEqual({
			address: { city: { $hidden: true }, zip: "10115" },
		});
	});

	test("rows without the input or already hidden stay as they are", async () => {
		const { store, tables } = sessionStore();
		await store.putRun(record(1, { notes: "x" }));
		const before = await tables.read("runs", KEY);
		await store.hideField(KEY, "constructor");
		await store.hideField(KEY, "vendor_name");
		expect(await tables.read("runs", KEY)).toEqual(before);
	});
});

describe("prefs", () => {
	test("the larger nextRunNumber wins, so two windows never reuse a number", async () => {
		const { store } = sessionStore();
		await store.putPrefs(
			KEY,
			prefs({ nextRunNumber: 10, perRun: ["invoice_file"] }),
		);
		await store.putPrefs(KEY, prefs({ nextRunNumber: 5, perRun: [] }));
		const loaded = await store.load(KEY);
		expect(loaded.prefs?.nextRunNumber).toBe(10);
		expect(loaded.prefs?.perRun).toEqual([]);
		await store.putPrefs(KEY, prefs({ nextRunNumber: 12 }));
		expect((await store.load(KEY)).prefs?.nextRunNumber).toBe(12);
	});

	test("writes from two windows at once keep the larger number", async () => {
		const { store } = sessionStore();
		await Promise.all([
			store.putPrefs(KEY, prefs({ nextRunNumber: 7 })),
			store.putPrefs(KEY, prefs({ nextRunNumber: 3 })),
		]);
		expect((await store.load(KEY)).prefs?.nextRunNumber).toBe(7);
	});

	test("settings of names unseen for 90 days are pruned; unstamped names stay", async () => {
		const { store } = sessionStore();
		const stale = NOW - 91 * DAY;
		const fresh = NOW - DAY;
		await store.putPrefs(
			KEY,
			prefs({
				perRun: ["old_date", "invoice_date", "unstamped"],
				noSave: ["old_date"],
				forgotten: { old_date: ["abc"], vendor_name: ["def"] },
				fieldSeenAt: {
					old_date: stale,
					invoice_date: fresh,
					vendor_name: fresh,
				},
			}),
		);
		const loaded = await store.load(KEY);
		expect(loaded.prefs?.perRun).toEqual(["invoice_date", "unstamped"]);
		expect(loaded.prefs?.noSave).toEqual([]);
		expect(loaded.prefs?.forgotten).toEqual({ vendor_name: ["def"] });
		expect(Object.keys(loaded.prefs?.fieldSeenAt ?? {})).toEqual([
			"invoice_date",
			"vendor_name",
		]);
	});

	test("prunePrefs leaves recent prefs as they are", () => {
		const recent = prefs({ fieldSeenAt: { a: NOW } });
		expect(prunePrefs(recent, NOW)).toBe(recent);
	});

	test("damaged parts of stored prefs read as empty", () => {
		const read = prefsOf(
			JSON.stringify({
				version: 2,
				perRun: ["a", 3],
				forgotten: { a: ["x"], b: "nope" },
				nextRunNumber: "7",
				fieldSeenAt: { a: 1, b: "x" },
			}),
		);
		expect(read).toEqual(
			prefs({ forgotten: { a: ["x"] }, fieldSeenAt: { a: 1 } }),
		);
		expect(prefsOf(JSON.stringify({ version: 1 }))).toBeNull();
		expect(prefsOf("{")).toBeNull();
	});
});

describe("presets", () => {
	test("presets load in the order they were made, and delete by id", async () => {
		const { store } = sessionStore();
		await store.putPreset(
			KEY,
			preset("nordwind", { createdAt: 2000, digit: 2 }),
		);
		await store.putPreset(
			KEY,
			preset("alpenfracht", { createdAt: 1000, digit: 1 }),
		);
		expect((await store.load(KEY)).presets.map((p) => p.id)).toEqual([
			"alpenfracht",
			"nordwind",
		]);
		await store.deletePreset(KEY, "alpenfracht");
		expect((await store.load(KEY)).presets.map((p) => p.id)).toEqual([
			"nordwind",
		]);
	});

	test(`a preset over ${FORM_LIMITS.presetBytes} bytes leaves out its largest values`, () => {
		const big = preset("big", {
			sets: {
				notes: "x".repeat(20_000),
				vendor_name: "Nordwind",
				max_pages: 60,
			},
			kinds: { notes: "text", vendor_name: "text", max_pages: "number" },
		});
		const fitted = fitPreset(big);
		expect(fitted.sets).toEqual({ vendor_name: "Nordwind", max_pages: 60 });
		expect(fitted.kinds).toEqual({ vendor_name: "text", max_pages: "number" });
		expect(JSON.stringify(fitted).length).toBeLessThanOrEqual(
			FORM_LIMITS.presetBytes,
		);
		const small = preset("small");
		expect(fitPreset(small)).toBe(small);
	});
});

describe("secrets never reach storage", () => {
	test("inputs named like secrets or holding key-like text are written hidden", async () => {
		const { store } = sessionStore();
		await store.putRun(
			record(1, {
				api_key: "plain",
				notes: "sk-live-abcdef",
				vendor_name: "Nordwind Logistik GmbH",
				invoice_file: { $file: { name: "a1".repeat(20), size: 1 } },
				tags: ["ok", "ghp_abcdef"],
				address: { city: "Berlin", token: { $hidden: true } },
				comment: { $hidden: true },
			}),
		);
		const [run] = (await store.load(KEY)).runs;
		expect(run.inputs).toEqual({
			api_key: { $hidden: true },
			notes: { $hidden: true },
			vendor_name: "Nordwind Logistik GmbH",
			invoice_file: { $file: { name: "a1".repeat(20), size: 1 } },
			tags: { $hidden: true },
			address: { city: "Berlin", token: { $hidden: true } },
			comment: { $hidden: true },
		});
	});

	test("preset values follow the same rules", async () => {
		const { store } = sessionStore();
		await store.putPreset(
			KEY,
			preset("p", { sets: { password: "hunter2", vendor_name: "Nordwind" } }),
		);
		const [stored] = (await store.load(KEY)).presets;
		expect(stored.sets).toEqual({
			password: { $hidden: true },
			vendor_name: "Nordwind",
		});
	});

	test("withoutSecrets leaves everyday inputs untouched", () => {
		const inputs = {
			vendor_name: "Alpenfracht AG",
			max_pages: 60,
			run_ocr: true,
		};
		expect(withoutSecrets(inputs)).toEqual(inputs);
	});
});

describe("damaged rows", () => {
	test("rows from another version or with broken JSON are skipped", async () => {
		const { store, tables } = sessionStore();
		await store.putRun(record(2));
		await tables.write("runs", KEY, [
			{ id: "broken", createdAt: 5000, json: "{" },
			{
				id: "old",
				createdAt: 4000,
				json: JSON.stringify({ ...record(4), version: 1 }),
			},
			{
				id: "odd",
				createdAt: 3000,
				json: JSON.stringify({ ...record(3), status: "weird" }),
			},
		]);
		await tables.write("presets", KEY, [{ id: "x", createdAt: 1, json: "[]" }]);
		const loaded = await store.load(KEY);
		expect(loaded.runs.map((run) => run.n)).toEqual([2]);
		expect(loaded.presets).toEqual([]);
		expect(runOf(JSON.stringify(record(7)))?.n).toBe(7);
		expect(runOf("null")).toBeNull();
	});
});

describe("storage errors are swallowed", () => {
	const failing = (fail: () => never): FormMemoryTables => ({
		read: async () => fail(),
		runIdsOldestFirst: async () => fail(),
		write: async () => fail(),
		remove: async () => fail(),
		transaction: async () => fail(),
	});

	test("every method resolves and the first error is reported once", async () => {
		const spy = quietWarnings();
		const store = createMemoryStoreOn(
			failing(() => {
				throw new Error("storage blocked");
			}),
			"device",
		);
		expect(await store.load(KEY)).toEqual({
			prefs: null,
			presets: [],
			runs: [],
		});
		await store.putRun(record(1));
		await store.deleteRun(KEY, "run-1");
		await store.hideField(KEY, "vendor_name");
		await store.putPrefs(KEY, prefs());
		await store.putPreset(KEY, preset("p"));
		await store.deletePreset(KEY, "p");
		expect(spy).toHaveBeenCalledTimes(1);
	});

	test("tables that throw synchronously are swallowed too", async () => {
		quietWarnings();
		const throwing: FormMemoryTables = {
			...failing(() => {
				throw new Error("async");
			}),
			read: () => {
				throw new Error("sync");
			},
			transaction: () => {
				throw new Error("sync");
			},
		};
		const store = createMemoryStoreOn(throwing, "device");
		expect((await store.load(KEY)).runs).toEqual([]);
		await store.putRun(record(1));
	});

	test("the device store works without IndexedDB: nothing saved, nothing thrown", async () => {
		quietWarnings();
		const store = createFormMemoryStore("device", null);
		expect(store.mode).toBe("device");
		expect(await store.load(KEY)).toEqual({
			prefs: null,
			presets: [],
			runs: [],
		});
		await store.putRun(record(1));
		await store.putPrefs(KEY, prefs());
	});
});

describe("session memory", () => {
	test("a given map holds the page session's memory, and another map is another session", async () => {
		const scope = new Map<string, unknown>();
		await createFormMemoryStore("session", scope).putRun(record(1));
		expect(
			(await createFormMemoryStore("session", scope).load(KEY)).runs,
		).toHaveLength(1);
		const other = createFormMemoryStore("session", new Map());
		expect((await other.load(KEY)).runs).toEqual([]);
		expect(scope.size).toBe(1);
	});

	test("without a map, stores of this page share one module map", async () => {
		const key = { ...KEY, eventId: `page-${Math.random()}` };
		await createFormMemoryStore("session", null).putRun({
			...record(9),
			...key,
		});
		const loaded = await createFormMemoryStore("session", null).load(key);
		expect(loaded.runs.map((run) => run.n)).toEqual([9]);
	});

	test("cleared memory, as when a runtime view closes, starts empty", async () => {
		const scope = new Map<string, unknown>();
		const store = createFormMemoryStore("session", scope);
		await store.putRun(record(1));
		scope.clear();
		expect((await store.load(KEY)).runs).toEqual([]);
	});
});
