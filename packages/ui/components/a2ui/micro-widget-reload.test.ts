import { describe, expect, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import type { AppPackageWidget } from "../../lib/package-widgets";
import {
	findMicroWidgetUpdate,
	isMicroWidgetReloadClean,
	migrateMicroWidgetProps,
	reloadMicroWidgetInstance,
} from "./micro-widget-reload";
import type { MicroWidgetInstanceComponent } from "./types";

const OLD_HASH = "a".repeat(64);
const NEW_HASH = "b".repeat(64);

function contract(
	inputs: WidgetContract["inputs"],
	events: WidgetContract["events"] = {},
): WidgetContract {
	return { contractVersion: 1, id: "map", inputs, events, queries: {} };
}

function installed(
	next: WidgetContract,
	overrides: Partial<AppPackageWidget> = {},
): AppPackageWidget {
	return {
		packageId: "geo",
		packageName: "Geo",
		packageVersion: "1.0.0",
		bundleHash: NEW_HASH,
		widget: {
			id: "map",
			name: "Map",
			description: "",
			icon: null,
			thumbnail: null,
			contract: next,
			keywords: [],
		},
		...overrides,
	};
}

function placed(
	previous: WidgetContract,
	overrides: Partial<MicroWidgetInstanceComponent> = {},
): MicroWidgetInstanceComponent {
	return {
		id: "microWidgetInstance-1",
		type: "microWidgetInstance",
		instanceId: "micro-map-1",
		packageId: "geo",
		widgetId: "map",
		packageVersion: "1.0.0",
		bundleHash: OLD_HASH,
		contract: previous,
		props: {},
		actionBindings: {},
		...overrides,
	};
}

describe("findMicroWidgetUpdate", () => {
	const current = contract({});

	test("a local rebuild changes only the bundle hash", () => {
		const update = installed(current);
		expect(findMicroWidgetUpdate([update], placed(current))).toBe(update);
	});

	test("a registry update changes the version", () => {
		const update = installed(current, {
			bundleHash: OLD_HASH,
			packageVersion: "1.1.0",
		});
		expect(findMicroWidgetUpdate([update], placed(current))).toBe(update);
	});

	test("the placed build is current", () => {
		const update = installed(current, { bundleHash: OLD_HASH });
		expect(findMicroWidgetUpdate([update], placed(current))).toBeNull();
	});

	test("hosts without bundle hashes compare versions only", () => {
		const update = installed(current, { bundleHash: undefined });
		expect(findMicroWidgetUpdate([update], placed(current))).toBeNull();
	});

	test("an instance without a hash takes the installed one", () => {
		const update = installed(current, { bundleHash: OLD_HASH });
		expect(
			findMicroWidgetUpdate([update], placed(current, { bundleHash: null })),
		).toBe(update);
	});

	test("another widget of the package or no install is not an update", () => {
		const other = installed(current);
		other.widget = { ...other.widget, id: "chart" };
		expect(findMicroWidgetUpdate([other], placed(current))).toBeNull();
		expect(findMicroWidgetUpdate(undefined, placed(current))).toBeNull();
	});
});

describe("migrateMicroWidgetProps", () => {
	test("keeps configured values the new input accepts", () => {
		const previous = contract({ title: { type: "string", default: "Map" } });
		const next = contract({ title: { type: "string", default: "Map" } });
		const result = migrateMicroWidgetProps({ title: "Sites" }, previous, next);
		expect(result.props).toEqual({ title: "Sites" });
		expect(result.report).toEqual({
			converted: [],
			defaulted: [],
			reset: [],
			removed: [],
		});
	});

	test("values still on the old default follow the new default", () => {
		const previous = contract({ zoom: { type: "number", default: 4 } });
		const next = contract({ zoom: { type: "number", default: 6 } });
		expect(migrateMicroWidgetProps({ zoom: 4 }, previous, next)).toEqual({
			props: { zoom: 6 },
			report: { converted: [], defaulted: ["zoom"], reset: [], removed: [] },
		});
		expect(migrateMicroWidgetProps({ zoom: 9 }, previous, next).props).toEqual({
			zoom: 9,
		});
	});

	test("converts losslessly between input types and clamps changed ranges", () => {
		const previous = contract({
			count: { type: "string" },
			label: { type: "number" },
			live: { type: "string" },
			mode: { type: "string" },
			level: { type: "number" },
			config: { type: "string" },
		});
		const next = contract({
			count: { type: "integer" },
			label: { type: "string" },
			live: { type: "boolean" },
			mode: { type: "enum", choices: ["light", "dark"] },
			level: { type: "integer", min: 1, max: 5 },
			config: {
				type: "json",
				schema: { type: "object", properties: { a: { type: "number" } } },
			},
		});
		const result = migrateMicroWidgetProps(
			{
				count: "12",
				label: 3,
				live: "true",
				mode: "Dark",
				level: 9,
				config: '{"a":1}',
			},
			previous,
			next,
		);
		expect(result.props).toEqual({
			count: 12,
			label: "3",
			live: true,
			mode: "dark",
			level: 5,
			config: { a: 1 },
		});
		expect(result.report.converted).toEqual([
			"count",
			"label",
			"live",
			"mode",
			"level",
			"config",
		]);
	});

	test("resets values that cannot be migrated and drops undeclared inputs", () => {
		const previous = contract({
			ratio: { type: "number" },
			theme: { type: "string" },
			legacy: { type: "string" },
		});
		const next = contract({
			ratio: { type: "integer", default: 1 },
			theme: { type: "enum", choices: ["light", "dark"] },
		});
		const result = migrateMicroWidgetProps(
			{ ratio: 1.5, theme: "sepia", legacy: "x" },
			previous,
			next,
		);
		expect(result.props).toEqual({ ratio: 1 });
		expect(result.report).toEqual({
			converted: [],
			defaulted: [],
			reset: ["ratio", "theme"],
			removed: ["legacy"],
		});
	});

	test("new inputs are seeded with their default and host props pass through", () => {
		const grants = [{ id: "radio", url: "https://radio.example.com/live" }];
		const result = migrateMicroWidgetProps(
			{ publicMediaGrants: grants },
			contract({}),
			contract({
				units: { type: "enum", choices: ["km", "mi"], default: "km" },
				caption: { type: "string" },
			}),
		);
		expect(result.props).toEqual({ publicMediaGrants: grants, units: "km" });
		expect(result.report.defaulted).toEqual(["units"]);
	});

	test("inherited object members are never treated as declared inputs", () => {
		const result = migrateMicroWidgetProps(
			{ toString: "x" },
			contract({}),
			contract({}),
		);
		expect(result.props).toEqual({});
		expect(result.report.removed).toEqual(["toString"]);
	});
});

describe("reloadMicroWidgetInstance", () => {
	test("moves the instance onto the installed build and keeps every route", () => {
		const previous = contract(
			{ title: { type: "string", default: "Map" } },
			{ selected: {}, moved: {} },
		);
		const next = contract(
			{ title: { type: "string", default: "Map" } },
			{
				selected: {
					payloadSchema: {
						type: "object",
						properties: { id: { type: "string" } },
					},
				},
			},
		);
		const component = placed(previous, {
			props: { title: "Sites" },
			actionBindings: { selected: { nodeId: "evt-1" } },
			eventHandlers: {
				moved: [{ name: "workflow_event", context: { nodeId: "evt-2" } }],
				"*": [],
			},
			actions: [{ name: "workflow_event", context: { nodeId: "evt-3" } }],
			style: { className: "h-64" },
		});

		const { component: reloaded, report } = reloadMicroWidgetInstance(
			component,
			installed(next, { packageVersion: "1.0.1" }),
		);

		expect(reloaded).toEqual({
			...component,
			packageVersion: "1.0.1",
			bundleHash: NEW_HASH,
			contract: next,
			props: { title: "Sites" },
		});
		expect(reloaded.contract).not.toBe(next);
		expect(report.undeclaredEvents).toEqual(["moved"]);
		expect(isMicroWidgetReloadClean(report)).toBe(false);
	});

	test("a rebuild with the same contract reports nothing", () => {
		const current = contract({ title: { type: "string", default: "Map" } });
		const { report } = reloadMicroWidgetInstance(
			placed(current, { props: { title: "Map" } }),
			installed(current),
		);
		expect(isMicroWidgetReloadClean(report)).toBe(true);
	});
});
