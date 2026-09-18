import { describe, expect, test } from "bun:test";
import {
	type WidgetConsentChip,
	buildWidgetConsentView,
	partitionWidgetChips,
} from "./micro-widget-consent-view";
import type {
	WidgetNetworkSource,
	WidgetPolicy,
	WidgetRuntimeSourceEntry,
	WidgetSourceLevel,
} from "./micro-widget-policy";
import type { MicroWidgetConsentPrompt } from "./use-micro-widget-grant";

function chip(
	host: string,
	level: WidgetSourceLevel,
	overrides: Partial<WidgetConsentChip> = {},
): WidgetConsentChip {
	const labels = host.split(".");
	return {
		key: host,
		host,
		wildcard: false,
		emphasis: labels.slice(-2).join("."),
		runtime: false,
		platformStorage: false,
		level,
		isNew: false,
		raised: false,
		included: true,
		...overrides,
	};
}

describe("chip collapse", () => {
	test("only known chips and repeats of a visible domain collapse", () => {
		const chips = [
			chip("fonts.googleapis.com", "known"),
			chip("fonts.gstatic.com", "known"),
			chip("tile.openstreetmap.org", "known"),
			chip("a.tile.openstreetmap.org", "known"),
			chip("b.tile.openstreetmap.org", "known"),
			chip("x.evil-attacker.net", "external"),
			chip("y.evil-attacker.net", "external"),
			chip("new.maps.example", "known", { isNew: true }),
		];
		const { visible, hidden } = partitionWidgetChips(chips, 4);
		expect(visible.map((entry) => entry.host)).toEqual([
			"fonts.googleapis.com",
			"fonts.gstatic.com",
			"x.evil-attacker.net",
			"new.maps.example",
		]);
		expect(hidden.map((entry) => entry.host)).toEqual([
			"tile.openstreetmap.org",
			"a.tile.openstreetmap.org",
			"b.tile.openstreetmap.org",
			"y.evil-attacker.net",
		]);
	});

	test("runtime chips never collapse", () => {
		const chips = [
			chip("a.example.com", "known"),
			chip("b.example.com", "known"),
			chip("r1.example.org", "external", { runtime: true }),
			chip("r2.example.org", "external", { runtime: true }),
		];
		expect(
			partitionWidgetChips(chips, 1).hidden.map((entry) => entry.host),
		).toEqual(["a.example.com", "b.example.com"]);
	});
});

const DIGEST = `sha256:${"a".repeat(64)}`;

function source(
	address: string,
	level: WidgetSourceLevel,
	overrides: Partial<WidgetNetworkSource> = {},
): WidgetNetworkSource {
	const host = address.slice(address.indexOf("://") + 3);
	return {
		source: address,
		directives: ["connectSrc"],
		origin: "declared",
		kind: "exact",
		level,
		host,
		emphasis: host.split(".").slice(-2).join("."),
		...overrides,
	};
}

function promptFor(
	purposes: { reason: string; sources: WidgetNetworkSource[] }[],
	overrides: Partial<MicroWidgetConsentPrompt> = {},
): MicroWidgetConsentPrompt {
	const all = purposes.flatMap((purpose) => purpose.sources);
	const policy: WidgetPolicy = {
		csp: { connectSrc: all.map((entry) => entry.source).sort() },
	};
	const runtime: WidgetRuntimeSourceEntry[] = all
		.filter((entry) => entry.origin === "runtime")
		.map((entry) => ({
			directive: "connectSrc",
			source: entry.source,
			level: entry.level,
			slot: entry.slot ?? null,
		}));
	const declared = all.filter((entry) => entry.origin === "declared");
	return {
		key: DIGEST,
		mode: "mount",
		descriptor: {
			source: "hub",
			packageId: "com.example.maps",
			bundleHash: "b",
			widgetId: "map",
			preview: false,
			status: "ok",
			policy,
			policyDigest: DIGEST,
			networkInputs: [],
			network: {
				level: "broad",
				catalogVersion: 1,
				pslVersion: "2026-09-15",
				stale: false,
				purposes: purposes.map((purpose) => ({
					reason: purpose.reason,
					level: "known",
					sources: purpose.sources,
				})),
			},
		},
		declaredDescriptor: null,
		subject: {
			source: "hub",
			packageId: "com.example.maps",
			widgetId: "map",
			policy,
		},
		declared: {
			policy: {
				csp: { connectSrc: declared.map((entry) => entry.source).sort() },
			},
			levels: Object.fromEntries(
				declared.map((entry) => [entry.source, entry.level]),
			),
			covered: false,
		},
		runtime: { pending: runtime, allowed: [], level: null, slots: [] },
		newSources: all.map((entry) => entry.source),
		raisedSources: [],
		newCapabilities: [],
		hasRuntimeCheckbox: runtime.length > 0,
		includeRuntimeDefault: true,
		includeRuntime: true,
		newerAvailable: false,
		canAllowForProject: true,
		canStopAsking: false,
		level: null,
		...overrides,
	};
}

describe("consent view", () => {
	const tiles = {
		reason: "Loads map tiles given to it at runtime",
		sources: [
			source("https://api.maps.example", "known", {
				kind: "service",
				provider: "Example Maps",
			}),
			source("https://tiles.webhook.site", "broad", {
				origin: "runtime",
				slot: "tileUrl",
				kind: "shared-host",
			}),
		],
	};

	test("an unchecked runtime part recomputes title, banner and tone from the declared part", () => {
		const checked = buildWidgetConsentView(promptFor([tiles]));
		expect(checked.title).toBe("broad");
		expect(checked.tone).toBe("broad");
		expect(checked.lead.kind).toBe("broad");

		const unchecked = buildWidgetConsentView(
			promptFor([tiles], { includeRuntime: false }),
		);
		expect(unchecked.title).toBe("network");
		expect(unchecked.tone).toBe("known");
		expect(unchecked.lead).toEqual({
			kind: "known",
			provider: "Example Maps",
			others: 0,
		});
		expect(unchecked.cards[0].runtime.map((entry) => entry.included)).toEqual([
			false,
		]);
	});

	test("cards sort by level, then new or raised, then declaration order", () => {
		const view = buildWidgetConsentView(
			promptFor(
				[
					{
						reason: "Fonts",
						sources: [source("https://f.fonts.example", "known")],
					},
					{
						reason: "Maps",
						sources: [source("https://a.maps.example", "external")],
					},
					{
						reason: "Data",
						sources: [source("https://b.data.example", "external")],
					},
				],
				{
					newSources: ["https://b.data.example"],
					raisedSources: [],
				},
			),
		);
		expect(view.expanded).toBe(true);
		expect(view.title).toBe("expanded");
		expect(view.cards.map((card) => card.reason)).toEqual([
			"Data",
			"Maps",
			"Fonts",
		]);
	});

	test("a runtime top source explains itself instead of a Risk line", () => {
		const view = buildWidgetConsentView(
			promptFor([
				{
					reason: "Tiles",
					sources: [
						source("https://a.tiles.example", "external", {
							origin: "runtime",
							slot: "tileUrl",
						}),
					],
				},
			]),
		);
		expect(view.cards[0].notes.map((note) => note.kind)).toEqual(["runtime"]);
		expect(view.cards[0].inputs).toEqual(["tileUrl"]);
	});

	test("without a classification every source is one broad unclassified card", () => {
		const base = promptFor([tiles]);
		const view = buildWidgetConsentView({
			...base,
			descriptor: base.descriptor
				? { ...base.descriptor, network: undefined }
				: null,
		});
		expect(view.classified).toBe(false);
		expect(view.cards).toHaveLength(1);
		expect(view.cards[0].reason).toBeNull();
		expect(view.networkLevel).toBe("broad");
		expect(view.lead.kind).toBe("unclassified");
	});

	test("a covered declared part only counts as already allowed", () => {
		const view = buildWidgetConsentView(
			promptFor([tiles], {
				declared: {
					policy: { csp: { connectSrc: ["https://api.maps.example"] } },
					levels: { "https://api.maps.example": "known" },
					covered: true,
				},
				hasRuntimeCheckbox: false,
				newSources: ["https://tiles.webhook.site"],
			}),
		);
		expect(view.alreadyAllowed).toBe(1);
		expect(view.cards[0].declared).toEqual([]);
		expect(view.cards[0].runtime.map((entry) => entry.isNew)).toEqual([true]);
		expect(view.capabilities).toEqual([]);
	});
});
