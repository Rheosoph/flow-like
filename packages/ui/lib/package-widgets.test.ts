import { describe, expect, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import {
	MicroWidgetPreviewLru,
	formatWidgetContractSummary,
	listAppPackageWidgets,
	readManifestWidgetBundleHash,
	readManifestWidgets,
	summarizeWidgetContract,
} from "./package-widgets";

const CONTRACT: WidgetContract = {
	contractVersion: 1,
	id: "sales-chart",
	inputs: {
		title: { type: "string", default: "Sales" },
		limit: { type: "number", default: 10 },
	},
	events: { "row-selected": {} },
	queries: {},
};

const WIDGET_ENTRY = {
	id: "sales-chart",
	name: "Sales Chart",
	description: "A chart",
	icon: null,
	thumbnail: "data:image/png;base64,xyz",
	contract: CONTRACT,
	keywords: ["chart", "sales"],
};

describe("readManifestWidgets", () => {
	test("reads valid entries and fills defaults", () => {
		const widgets = readManifestWidgets({ widgets: [WIDGET_ENTRY] });
		expect(widgets).toHaveLength(1);
		expect(widgets[0].id).toBe("sales-chart");
		expect(widgets[0].keywords).toEqual(["chart", "sales"]);
		expect(widgets[0].thumbnail).toBe("data:image/png;base64,xyz");
	});

	test("tolerates missing/invalid shapes", () => {
		expect(readManifestWidgets(undefined)).toEqual([]);
		expect(readManifestWidgets({})).toEqual([]);
		expect(readManifestWidgets({ widgets: "nope" })).toEqual([]);
		expect(
			readManifestWidgets({
				widgets: [
					null,
					{ id: "no-name", contract: CONTRACT },
					{ id: "no-contract", name: "X" },
					WIDGET_ENTRY,
				],
			}),
		).toHaveLength(1);
	});

	test("normalizes missing optional fields", () => {
		const widgets = readManifestWidgets({
			widgets: [{ id: "w", name: "W", contract: CONTRACT }],
		});
		expect(widgets[0].description).toBe("");
		expect(widgets[0].icon).toBeNull();
		expect(widgets[0].thumbnail).toBeNull();
		expect(widgets[0].keywords).toEqual([]);
	});
});

describe("readManifestWidgetBundleHash", () => {
	test("accepts camelCase and snake_case", () => {
		expect(readManifestWidgetBundleHash({ widgetBundleHash: "abc" })).toBe(
			"abc",
		);
		expect(readManifestWidgetBundleHash({ widget_bundle_hash: "def" })).toBe(
			"def",
		);
		expect(
			readManifestWidgetBundleHash({
				widgetBundleHash: "abc",
				widget_bundle_hash: "def",
			}),
		).toBe("abc");
		expect(readManifestWidgetBundleHash({})).toBeUndefined();
		expect(readManifestWidgetBundleHash(null)).toBeUndefined();
	});
});

describe("contract summary", () => {
	test("counts inputs, events and queries", () => {
		expect(summarizeWidgetContract(CONTRACT)).toEqual({
			inputs: 2,
			events: 1,
			queries: 0,
		});
		expect(summarizeWidgetContract(null)).toEqual({
			inputs: 0,
			events: 0,
			queries: 0,
		});
	});

	test("formats with correct pluralization", () => {
		expect(
			formatWidgetContractSummary({ inputs: 2, events: 1, queries: 0 }),
		).toBe("2 inputs · 1 event · 0 queries");
		expect(
			formatWidgetContractSummary({ inputs: 1, events: 0, queries: 1 }),
		).toBe("1 input · 0 events · 1 query");
	});
});

describe("MicroWidgetPreviewLru", () => {
	test("evicts the least recently used entry beyond capacity", () => {
		const lru = new MicroWidgetPreviewLru(2);
		const evicted: string[] = [];
		lru.activate("a", () => evicted.push("a"));
		lru.activate("b", () => evicted.push("b"));
		expect(lru.activate("c", () => evicted.push("c"))).toEqual(["a"]);
		expect(evicted).toEqual(["a"]);
		expect(lru.has("a")).toBe(false);
		expect(lru.has("b")).toBe(true);
		expect(lru.size).toBe(2);
	});

	test("re-activating an entry refreshes its position", () => {
		const lru = new MicroWidgetPreviewLru(2);
		const evicted: string[] = [];
		lru.activate("a", () => evicted.push("a"));
		lru.activate("b", () => evicted.push("b"));
		lru.activate("a", () => evicted.push("a"));
		lru.activate("c", () => evicted.push("c"));
		expect(evicted).toEqual(["b"]);
	});

	test("touch refreshes without replacing the callback", () => {
		const lru = new MicroWidgetPreviewLru(2);
		const evicted: string[] = [];
		lru.activate("a", () => evicted.push("a"));
		lru.activate("b", () => evicted.push("b"));
		lru.touch("a");
		lru.activate("c", () => evicted.push("c"));
		expect(evicted).toEqual(["b"]);
	});

	test("release frees a slot without invoking the callback", () => {
		const lru = new MicroWidgetPreviewLru(1);
		const evicted: string[] = [];
		lru.activate("a", () => evicted.push("a"));
		lru.release("a");
		expect(evicted).toEqual([]);
		expect(lru.size).toBe(0);
		lru.activate("b", () => evicted.push("b"));
		expect(evicted).toEqual([]);
	});
});

describe("listAppPackageWidgets", () => {
	const installed = {
		version: "1.2.3",
		manifest: {
			version: "1.2.3",
			name: "Example Pack",
			widgets: [WIDGET_ENTRY],
			widget_bundle_hash: "deadbeef",
		},
		metadata: { name: "Example Pack Meta" },
	};

	test("keeps the resolved version with its contract and bundle when the listed pin is stale", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => ({ "com.example.pack": "1.2.0" }),
				getPackage: async () => installed,
			},
			"app-1",
		);
		expect(result).toHaveLength(1);
		expect(result[0].packageId).toBe("com.example.pack");
		expect(result[0].packageName).toBe("Example Pack Meta");
		expect(result[0].packageVersion).toBe(installed.manifest.version);
		expect(result[0].bundleHash).toBe("deadbeef");
		expect(result[0].widget.id).toBe("sales-chart");
		expect(result[0].widget.contract).toEqual(WIDGET_ENTRY.contract);
	});

	test("resolves each package through the app so members see what it pins", async () => {
		const asked: [string, string][] = [];
		await listAppPackageWidgets(
			{
				listPackages: async () => ({ "com.example.pack": "1.2.0" }),
				getPackage: async (packageId, appId) => {
					asked.push([packageId, appId]);
					return installed;
				},
			},
			"app-1",
		);
		expect(asked).toEqual([["com.example.pack", "app-1"]]);
	});

	test("uses the resolved version when the pin is empty", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => ({ "com.example.pack": "" }),
				getPackage: async () => installed,
			},
			"app-1",
		);
		expect(result[0].packageVersion).toBe("1.2.3");
	});

	test("falls back to the listed pin when the resolved version is empty", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => ({ "com.example.pack": "1.2.0" }),
				getPackage: async () => ({ ...installed, version: "" }),
			},
			"app-1",
		);
		expect(result[0].packageVersion).toBe("1.2.0");
	});

	test("returns empty without listPackages support", async () => {
		const result = await listAppPackageWidgets(
			{ getPackage: async () => installed },
			"app-1",
		);
		expect(result).toEqual([]);
	});

	test("skips packages that fail to resolve or have no widgets", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => ({
					broken: "1.0.0",
					empty: "1.0.0",
					missing: "1.0.0",
				}),
				getPackage: async (id) => {
					if (id === "broken") throw new Error("boom");
					if (id === "missing") return null;
					return { version: "1.0.0", manifest: { widgets: [] } };
				},
			},
			"app-1",
		);
		expect(result).toEqual([]);
	});

	test("returns an empty list when listing packages fails by default", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => {
					throw new Error("Package listing unavailable");
				},
				getPackage: async () => installed,
			},
			"app-1",
		);
		expect(result).toEqual([]);
	});

	test("propagates package listing failures in strict mode", async () => {
		const failure = new Error("Package listing unavailable");
		let packageReads = 0;
		await expect(
			listAppPackageWidgets(
				{
					listPackages: async () => {
						throw failure;
					},
					getPackage: async () => {
						packageReads += 1;
						return installed;
					},
				},
				"app-1",
				{ strict: true },
			),
		).rejects.toBe(failure);
		expect(packageReads).toBe(0);
	});

	test("rejects partial package results when a manifest read fails in strict mode", async () => {
		const failure = new Error("Private package unavailable");
		await expect(
			listAppPackageWidgets(
				{
					listPackages: async () => ({
						"com.example.pack": "1.2.3",
						"com.example.private": "1.0.0",
					}),
					getPackage: async (id) => {
						if (id === "com.example.private") throw failure;
						return installed;
					},
				},
				"app-1",
				{ strict: true },
			),
		).rejects.toBe(failure);
	});

	test("identifies a listed package that cannot be loaded in strict mode", async () => {
		await expect(
			listAppPackageWidgets(
				{
					listPackages: async () => ({
						"com.example.pack": "1.2.3",
						"com.example.missing": "1.0.0",
					}),
					getPackage: async (id) =>
						id === "com.example.missing" ? null : installed,
				},
				"app-1",
				{ strict: true },
			),
		).rejects.toThrow("Package com.example.missing could not be loaded.");
	});

	test("allows installed packages without widgets in strict mode", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => ({
					"com.example.pack": "1.2.3",
					"com.example.nodes": "1.0.0",
				}),
				getPackage: async (id) =>
					id === "com.example.nodes"
						? { version: "1.0.0", manifest: { nodes: [] } }
						: installed,
			},
			"app-1",
			{ strict: true },
		);
		expect(result).toHaveLength(1);
		expect(result[0]).toMatchObject({
			packageId: "com.example.pack",
			packageVersion: "1.2.3",
			bundleHash: "deadbeef",
			widget: { id: WIDGET_ENTRY.id, contract: CONTRACT },
		});
	});

	test("resolves only the selected package even if another package is inaccessible", async () => {
		const reads: string[] = [];
		const widgets = await listAppPackageWidgets(
			{
				listPackages: async () => ({ selected: "1.2.3", unavailable: "1.0.0" }),
				getPackage: async (id) => {
					reads.push(id);
					if (id !== "selected") throw new Error("Package unavailable");
					return installed;
				},
			},
			"app-1",
			{ strict: true, packageId: "selected" },
		);
		expect(reads).toEqual(["selected"]);
		expect(widgets).toHaveLength(1);
		expect(widgets[0].packageId).toBe("selected");
	});

	test("refuses to update a selected package removed from the app", async () => {
		let reads = 0;
		await expect(
			listAppPackageWidgets(
				{
					listPackages: async () => ({ other: "1.0.0" }),
					getPackage: async () => {
						reads++;
						return installed;
					},
				},
				"app-1",
				{ strict: true, packageId: "removed" },
			),
		).rejects.toThrow("no longer added");
		expect(reads).toBe(0);
	});
});
