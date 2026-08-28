import { describe, expect, test } from "bun:test";
import {
	materializeSurfaceElements,
	selectElements,
} from "./element-materializer";
import type { SurfaceComponent } from "./types";
import type { WidgetElementComponent } from "./workflow-elements";

const SURFACE = "page-1";

function element(id: string, data: Record<string, unknown>) {
	return { id, component: data };
}

function children(...ids: string[]) {
	return { children: { explicitList: ids } };
}

const widgetChildren: WidgetElementComponent[] = [
	{ id: "wrap", component: { type: "row", ...children("field", "only") } },
	{ id: "field", component: { type: "textField" } },
	{ id: "only", component: { type: "checkbox" } },
];

const host = element("host", {
	type: "widgetInstance",
	instanceId: "inst-1",
	widgetId: "shared-widget",
	inlineWidgetDef: { rootComponentId: "wrap", components: widgetChildren },
});

const all: Record<string, unknown> = {
	[`${SURFACE}/root`]: element("root", {
		type: "column",
		...children("title", "host", "a.b"),
	}),
	[`${SURFACE}/title`]: element("title", { type: "text" }),
	[`${SURFACE}/host`]: host,
	[`${SURFACE}/a.b`]: element("a.b", { type: "Text" }),
	[`${SURFACE}/axb`]: element("axb", { type: "button" }),
	[`${SURFACE}/field`]: element("field", { type: "textField" }),
	[`${SURFACE}/wrap`]: element("wrap", { type: "row", ...children("field") }),
	"inst-1/wrap": element("wrap", { type: "row", ...children("field", "only") }),
	"inst-1/field": element("field", { type: "textField" }),
	"inst-1/only": element("only", { type: "checkbox" }),
	"micro-1/values": {
		id: "values",
		component: { value: { literalJson: '{"choice":"A"}' } },
	},
};

const keysOf = (selectors: string[]) =>
	Object.keys(selectElements(all, selectors, SURFACE)).sort();

describe("selectElements key resolution", () => {
	test("exact page key", () => {
		expect(keysOf([`${SURFACE}/title`])).toEqual([`${SURFACE}/title`]);
		expect(selectElements(all, [`${SURFACE}/title`], SURFACE)).toEqual({
			[`${SURFACE}/title`]: all[`${SURFACE}/title`],
		});
	});

	test("foreign page refs retarget to the current surface", () => {
		expect(keysOf(["page-9/title"])).toEqual([`${SURFACE}/title`]);
		expect(keysOf(["page-9/missing"])).toEqual([]);
	});

	test("never retargets a prefix that names a widget instance in the map", () => {
		expect(keysOf(["inst-1/title"])).toEqual([]);
		expect(keysOf(["micro-1/title"])).toEqual([]);
	});

	test("bare element ids prefer the surface, then any suffix match", () => {
		expect(keysOf(["title"])).toEqual([`${SURFACE}/title`]);
		expect(keysOf(["field"])).toEqual([`${SURFACE}/field`]);
		expect(keysOf(["only"])).toEqual(["inst-1/only"]);
		expect(keysOf(["nothing"])).toEqual([]);
	});

	test("widget children are addressed under their instance id", () => {
		expect(keysOf(["inst-1/field"])).toEqual(["inst-1/field"]);
		expect(keysOf(["inst-1/nothing"])).toEqual([]);
	});

	test("trims whitespace and ignores non-string selectors", () => {
		expect(keysOf([`  ${SURFACE}/title `])).toEqual([`${SURFACE}/title`]);
		expect(
			keysOf([undefined as unknown as string, 7 as unknown as string, ""]),
		).toEqual([]);
	});
});

describe("selectElements prefixed selectors", () => {
	test("host: returns the host element as-is with its inline definition", () => {
		const byKey = selectElements(all, [`host:${SURFACE}/host`], SURFACE);
		expect(byKey).toEqual({ [`${SURFACE}/host`]: host });
		const inline = (byKey[`${SURFACE}/host`] as Record<string, unknown>)
			.component as Record<string, unknown>;
		expect(inline.inlineWidgetDef).toBeDefined();

		expect(keysOf(["host:host"])).toEqual([`${SURFACE}/host`]);
		expect(keysOf(["host:page-9/host"])).toEqual([`${SURFACE}/host`]);
		expect(keysOf(["host:nothing"])).toEqual([]);
	});

	test("type: matches component.type case-insensitively", () => {
		expect(keysOf(["type:TEXT"])).toEqual([
			`${SURFACE}/a.b`,
			`${SURFACE}/title`,
		]);
		expect(keysOf(["type:textfield"])).toEqual([
			"inst-1/field",
			`${SURFACE}/field`,
		]);
		expect(keysOf(["type:nope"])).toEqual([]);
		expect(keysOf(["type:"])).toEqual([]);
	});

	test("glob: matches keys with * wildcards and escapes regex specials", () => {
		expect(keysOf(["glob:inst-1/*"])).toEqual([
			"inst-1/field",
			"inst-1/only",
			"inst-1/wrap",
		]);
		expect(keysOf(["glob:*/field"])).toEqual([
			"inst-1/field",
			`${SURFACE}/field`,
		]);
		expect(keysOf([`glob:${SURFACE}/a.b`])).toEqual([`${SURFACE}/a.b`]);
		expect(keysOf(["glob:*"]).length).toBe(Object.keys(all).length);
		expect(keysOf(["glob:zzz*"])).toEqual([]);
		expect(keysOf(["glob:"])).toEqual([]);
		expect(keysOf(["glob:(["])).toEqual([]);
	});

	test("children: returns the container and its explicit children", () => {
		expect(keysOf([`children:${SURFACE}/root`])).toEqual([
			`${SURFACE}/a.b`,
			`${SURFACE}/host`,
			`${SURFACE}/root`,
			`${SURFACE}/title`,
		]);
		expect(keysOf(["children:root"])).toEqual([
			`${SURFACE}/a.b`,
			`${SURFACE}/host`,
			`${SURFACE}/root`,
			`${SURFACE}/title`,
		]);
		expect(keysOf(["children:title"])).toEqual([`${SURFACE}/title`]);
		expect(keysOf(["children:nothing"])).toEqual([]);
	});

	test("children: of a widget container stays inside the instance", () => {
		expect(keysOf(["children:inst-1/wrap"])).toEqual([
			"inst-1/field",
			"inst-1/only",
			"inst-1/wrap",
		]);
	});

	test("parent: finds the container listing the element", () => {
		expect(keysOf([`parent:${SURFACE}/title`])).toEqual([`${SURFACE}/root`]);
		expect(keysOf(["parent:title"])).toEqual([`${SURFACE}/root`]);
		expect(keysOf(["parent:page-9/title"])).toEqual([`${SURFACE}/root`]);
		expect(keysOf(["parent:inst-1/field"])).toEqual(["inst-1/wrap"]);
		expect(keysOf(["parent:field"])).toEqual([`${SURFACE}/wrap`]);
		expect(keysOf(["parent:inst-1/only"])).toEqual(["inst-1/wrap"]);
		expect(keysOf([`parent:${SURFACE}/root`])).toEqual([]);
		expect(keysOf(["parent:nothing"])).toEqual([]);
	});

	test("values: returns the micro widget value mirror", () => {
		expect(selectElements(all, ["values:micro-1"], SURFACE)).toEqual({
			"micro-1/values": all["micro-1/values"],
		});
		expect(keysOf(["values:inst-1"])).toEqual([]);
		expect(keysOf(["values:"])).toEqual([]);
	});

	test("unknown prefixes contribute nothing", () => {
		expect(keysOf([`foo:${SURFACE}/title`, "bar:*", "HOST:title"])).toEqual([]);
	});

	test("selectors never throw on malformed maps", () => {
		const weird: Record<string, unknown> = {
			"page-1/null": null,
			"page-1/list": [1, 2],
			"page-1/bad-children": element("bad-children", {
				type: "column",
				children: { explicitList: "field" },
			}),
			"page-1/no-component": { id: "no-component" },
			"page-1/": element("", { type: "text" }),
		};
		expect(() =>
			selectElements(
				weird,
				[
					"null",
					"page-1/list",
					"children:bad-children",
					"parent:field",
					"type:text",
					"glob:*",
					"no-component",
					"page-1/",
					"/",
					"page-9/",
				],
				"page-1",
			),
		).not.toThrow();
		expect(Object.keys(selectElements(weird, ["type:text"], "page-1"))).toEqual(
			["page-1/"],
		);
	});

	test("each entry is included at most once across overlapping selectors", () => {
		const selected = selectElements(
			all,
			[`${SURFACE}/title`, "title", "type:text", "glob:*/title", "host:title"],
			SURFACE,
		);
		expect(Object.keys(selected).sort()).toEqual([
			`${SURFACE}/a.b`,
			`${SURFACE}/title`,
		]);
	});

	test("an empty selector list yields an empty map", () => {
		expect(selectElements(all, [], SURFACE)).toEqual({});
		expect(selectElements({}, ["title", "type:text"], SURFACE)).toEqual({});
	});
});

describe("materializeSurfaceElements", () => {
	function component(
		id: string,
		data: Record<string, unknown>,
		style?: unknown,
	): SurfaceComponent {
		return { id, component: data, style } as unknown as SurfaceComponent;
	}

	const components: Record<string, SurfaceComponent> = {
		root: component("root", { type: "column", ...children("host", "micro") }),
		host: component(
			"host",
			{
				type: "widgetInstance",
				instanceId: "inst-1",
				widgetId: "shared-widget",
				inlineWidgetDef: {
					rootComponentId: "field",
					components: [
						{
							id: "field",
							eventRelevant: true,
							component: { type: "textField", value: { literalString: "" } },
						},
					],
				},
			},
			{ className: "host", background: null },
		),
		micro: component("micro", {
			type: "microWidgetInstance",
			instanceId: "micro-1",
		}),
	};

	const storedValues = {
		"inst-1/field": "typed",
		"micro-1/values": { choice: "A" },
		"other-page/field": "foreign",
	};

	test("builds the merged map and applies the selectors", () => {
		const elements = materializeSurfaceElements(
			{ surfaceId: SURFACE, components, storedValues },
			["host:host", "inst-1/field", "values:micro-1", "children:root"],
		);

		expect(Object.keys(elements).sort()).toEqual([
			"inst-1/field",
			"micro-1/values",
			`${SURFACE}/host`,
			`${SURFACE}/micro`,
			`${SURFACE}/root`,
		]);

		const hostElement = elements[`${SURFACE}/host`] as Record<string, unknown>;
		expect(hostElement.style).toEqual({ className: "host" });
		const inline = (hostElement.component as Record<string, unknown>)
			.inlineWidgetDef as Record<string, unknown>;
		expect(inline.rootComponentId).toBe("field");

		const field = elements["inst-1/field"] as Record<string, unknown>;
		expect((field.component as Record<string, unknown>).value).toEqual({
			literalString: "typed",
		});

		const values = elements["micro-1/values"] as Record<string, unknown>;
		expect((values.component as Record<string, unknown>).value).toEqual({
			literalJson: '{"choice":"A"}',
		});
	});

	test("resolves foreign and bare refs against the live surface", () => {
		const elements = materializeSurfaceElements(
			{ surfaceId: SURFACE, components, storedValues },
			["other-page/root", "field", "type:widgetinstance"],
		);
		expect(Object.keys(elements).sort()).toEqual([
			`${SURFACE}/field`,
			`${SURFACE}/host`,
			`${SURFACE}/root`,
		]);
		expect(elements["other-page/field"]).toBeUndefined();
	});

	test("returns an empty map without components or without a surface", () => {
		expect(
			materializeSurfaceElements(
				{ surfaceId: SURFACE, components: undefined, storedValues },
				["glob:*"],
			),
		).toEqual({});
		expect(
			materializeSurfaceElements({ surfaceId: "", components, storedValues }, [
				"glob:*",
			]),
		).toEqual({});
	});
});

describe("materializeSurfaceElements with a widget scope", () => {
	function widgetHost(
		hostId: string,
		instanceId: string,
		childId: string,
	): SurfaceComponent {
		return {
			id: hostId,
			component: {
				type: "widgetInstance",
				instanceId,
				widgetId: "shared-widget",
				inlineWidgetDef: {
					rootComponentId: childId,
					components: [
						{
							id: childId,
							eventRelevant: true,
							component: { type: "textField", value: { literalString: "" } },
						},
					],
				},
			},
		} as unknown as SurfaceComponent;
	}

	const components: Record<string, SurfaceComponent> = {
		root: {
			id: "root",
			component: { type: "column", ...children("first", "second") },
		} as unknown as SurfaceComponent,
		first: widgetHost("first", "inst-1", "field"),
		second: widgetHost("second", "inst-2", "field"),
	};
	const storedValues = {
		"inst-1/field": "one",
		"inst-2/field": "two",
	};
	const scope = { instanceId: "inst-1" };

	test("selects from the instance-addressed map of the active widget only", () => {
		const elements = materializeSurfaceElements(
			{ surfaceId: SURFACE, components, storedValues },
			["glob:*"],
			scope,
		);
		expect(Object.keys(elements).sort()).toEqual([
			"inst-1/field",
			`${SURFACE}/first`,
			`${SURFACE}/root`,
		]);
		const field = elements["inst-1/field"] as Record<string, unknown>;
		expect((field.component as Record<string, unknown>).value).toEqual({
			literalString: "one",
		});
	});

	test("resolves the triggering child and bare ids inside the instance", () => {
		const elements = materializeSurfaceElements(
			{ surfaceId: SURFACE, components, storedValues },
			["inst-1/field", "field", "inst-2/field"],
			scope,
		);
		expect(Object.keys(elements)).toEqual(["inst-1/field"]);
	});

	test("without a scope the same selectors reach every instance", () => {
		const elements = materializeSurfaceElements(
			{ surfaceId: SURFACE, components, storedValues },
			["inst-1/field", "inst-2/field"],
		);
		expect(Object.keys(elements).sort()).toEqual([
			"inst-1/field",
			"inst-2/field",
		]);
	});
});

describe("foreign-page refs mirror the runtime's third resolution step", () => {
	test("falls back to any key with the element suffix when the page has no twin", () => {
		const all = {
			"page/root": { component: { type: "column" } },
			"inst-1/field": { component: { type: "textField" } },
		};
		expect(
			Object.keys(selectElements(all, ["other-page/field"], "page")),
		).toEqual(["inst-1/field"]);
		expect(
			Object.keys(selectElements(all, ["other-page/root"], "page")),
		).toEqual(["page/root"]);
		expect(selectElements(all, ["inst-1/root"], "page")).toEqual({});
	});
});
