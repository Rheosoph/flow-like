import { describe, expect, test } from "bun:test";
import {
	MicroWidgetPreviewLru,
	formatWidgetContractSummary,
	listAppPackageWidgets,
	readManifestWidgetBundleHash,
	readManifestWidgets,
	summarizeWidgetContract,
} from "./package-widgets";

const CONTRACT = {
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
			name: "Example Pack",
			widgets: [WIDGET_ENTRY],
			widget_bundle_hash: "deadbeef",
		},
		metadata: { name: "Example Pack Meta" },
	};

	test("resolves widgets from installed manifests of app packages", async () => {
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
		expect(result[0].packageVersion).toBe("1.2.0");
		expect(result[0].bundleHash).toBe("deadbeef");
		expect(result[0].widget.id).toBe("sales-chart");
	});

	test("falls back to the installed version when the pin is empty", async () => {
		const result = await listAppPackageWidgets(
			{
				listPackages: async () => ({ "com.example.pack": "" }),
				getPackage: async () => installed,
			},
			"app-1",
		);
		expect(result[0].packageVersion).toBe("1.2.3");
	});

	test("returns empty without listPackages support (web wiring gap)", async () => {
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
});
