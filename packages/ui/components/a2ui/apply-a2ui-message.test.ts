/**
 * Tests for the shared a2ui reducer, focused on the setGeoMapViewport
 * normalization (backend emits shapes the GeoMap component cannot resolve
 * without it).
 */
import { describe, expect, test } from "bun:test";
import {
	applyA2UIMessage,
	applyElementUpdate,
	applyMicroWidgetPropsPatch,
	normalizeGeoMapViewport,
} from "./apply-a2ui-message";
import type { Surface, SurfaceComponent } from "./types";

const geoMapComponent = (): SurfaceComponent =>
	({
		id: "map-1",
		component: { type: "geoMap" },
	}) as unknown as SurfaceComponent;

describe("normalizeGeoMapViewport", () => {
	test("wraps the node's legacy flat shape into a nested-center literalJson", () => {
		const result = normalizeGeoMapViewport({
			latitude: 52.52,
			longitude: 13.405,
			zoom: 10,
		}) as { literalJson: string };

		expect(result.literalJson).toBeString();
		expect(JSON.parse(result.literalJson)).toEqual({
			center: { latitude: 52.52, longitude: 13.405 },
			zoom: 10,
		});
	});

	test("passes BoundValues through untouched", () => {
		const bound = { literalJson: '{"center":{"latitude":1,"longitude":2}}' };
		expect(normalizeGeoMapViewport(bound)).toBe(bound);

		const path = { path: "/inputs/viewport" };
		expect(normalizeGeoMapViewport(path)).toBe(path);
	});

	test("wraps an already-nested raw object", () => {
		const result = normalizeGeoMapViewport({
			center: { latitude: 1, longitude: 2 },
			bearing: 45,
		}) as { literalJson: string };

		expect(JSON.parse(result.literalJson)).toEqual({
			center: { latitude: 1, longitude: 2 },
			bearing: 45,
		});
	});

	test("returns undefined for unusable payloads", () => {
		expect(normalizeGeoMapViewport(undefined)).toBeUndefined();
		expect(normalizeGeoMapViewport({ zoom: 3 })).toBeUndefined();
	});
});

describe("applyElementUpdate setGeoMapViewport", () => {
	test("stores a resolvable BoundValue on the component", () => {
		const updated = applyElementUpdate(geoMapComponent(), {
			type: "setGeoMapViewport",
			viewport: { latitude: 48.13, longitude: 11.58, zoom: 12 },
		});

		const viewport = (updated.component as unknown as Record<string, unknown>)
			.viewport as { literalJson: string };
		expect(JSON.parse(viewport.literalJson).center).toEqual({
			latitude: 48.13,
			longitude: 11.58,
		});
	});
});

describe("applyElementUpdate event actions", () => {
	const button = (): SurfaceComponent =>
		({
			id: "button-1",
			component: {
				type: "button",
				label: { literalString: "Open" },
				actions: [{ name: "workflow_event", context: { nodeId: "legacy" } }],
				eventHandlers: {
					hover: [{ name: "workflow_event", context: { nodeId: "hover" } }],
				},
			},
		}) as unknown as SurfaceComponent;

	test("sets one named ordered action list without touching legacy actions", () => {
		const actions = [
			{ name: "workflow_event", context: { nodeId: "first" } },
			{ name: "navigate_page", context: { route: "/done" } },
		];
		const updated = applyElementUpdate(button(), {
			type: "setEventActions",
			eventName: " click ",
			actions,
		});
		const data = updated.component as unknown as Record<string, unknown>;

		expect(data.actions).toEqual([
			{ name: "workflow_event", context: { nodeId: "legacy" } },
		]);
		expect(data.eventHandlers).toEqual({
			hover: [{ name: "workflow_event", context: { nodeId: "hover" } }],
			click: actions,
		});
	});

	test("an empty list explicitly disables a named event", () => {
		const updated = applyElementUpdate(button(), {
			type: "setEventActions",
			eventName: "click",
			actions: [],
		});
		const data = updated.component as unknown as Record<string, unknown>;
		expect(data.eventHandlers).toEqual({
			hover: [{ name: "workflow_event", context: { nodeId: "hover" } }],
			click: [],
		});
	});

	test("ignores malformed named-event updates", () => {
		const original = button();
		expect(
			applyElementUpdate(original, {
				type: "setEventActions",
				eventName: "",
				actions: [],
			}),
		).toBe(original);
	});
});

describe("applyElementUpdate microWidgetInstance props patches", () => {
	const microComponent = (): SurfaceComponent =>
		({
			id: "inst-1",
			component: {
				type: "microWidgetInstance",
				instanceId: "inst-1",
				packageId: "com.example.sales",
				widgetId: "sales-chart",
				packageVersion: "1.0.0",
				props: { title: "Sales", limit: 50 },
			},
		}) as unknown as SurfaceComponent;

	const dataOf = (component: SurfaceComponent) =>
		component.component as unknown as Record<string, unknown>;

	test("setProps merges into component.props, never onto the component itself", () => {
		const updated = applyElementUpdate(microComponent(), {
			type: "setProps",
			props: { title: "Q3 Sales", variant: "line" },
		});
		const data = dataOf(updated);
		expect(data.props).toEqual({
			title: "Q3 Sales",
			limit: 50,
			variant: "line",
		});
		expect(data.title).toBeUndefined();
		expect(data.instanceId).toBe("inst-1");
	});

	test("a typed patch with a props object merges regardless of the type name", () => {
		const updated = applyElementUpdate(microComponent(), {
			type: "updateWidgetInputs",
			props: { limit: 10 },
		});
		expect(dataOf(updated).props).toEqual({ title: "Sales", limit: 10 });
	});

	test("an untyped flat patch merges into props", () => {
		const updated = applyElementUpdate(microComponent(), { title: "Renamed" });
		expect(dataOf(updated).props).toEqual({ title: "Renamed", limit: 50 });
	});

	test("applyMicroWidgetPropsPatch ignores non-micro components", () => {
		expect(
			applyMicroWidgetPropsPatch({ type: "text" }, { props: { a: 1 } }),
		).toBeNull();
	});

	test("styling updates fall through to the generic handling", () => {
		const updated = applyElementUpdate(microComponent(), {
			type: "setVisibility",
			visible: false,
		});
		const data = dataOf(updated);
		expect(data.hidden).toEqual({ literalBool: true });
		expect(data.props).toEqual({ title: "Sales", limit: 50 });
	});

	test("initializes props when the component has none yet", () => {
		const bare = {
			id: "inst-2",
			component: { type: "microWidgetInstance", instanceId: "inst-2" },
		} as unknown as SurfaceComponent;
		const updated = applyElementUpdate(bare, {
			type: "setProps",
			props: { title: "Hello" },
		});
		expect(dataOf(updated).props).toEqual({ title: "Hello" });
	});
});

describe("value writes", () => {
	const dataOf = (component: SurfaceComponent) =>
		component.component as unknown as Record<string, unknown>;

	const inputComponent = (data: Record<string, unknown>): SurfaceComponent =>
		({ id: "field-1", component: data }) as unknown as SurfaceComponent;

	test("setValue advances the revision even when the value is unchanged", () => {
		const component = inputComponent({
			type: "textField",
			value: { literalString: "" },
		});

		const cleared = applyElementUpdate(component, {
			type: "setValue",
			value: "",
		});

		expect(dataOf(cleared).value).toEqual({ literalString: "" });
		expect(dataOf(cleared).valueRevision).toBe(1);

		const again = applyElementUpdate(cleared, { type: "setValue", value: "" });
		expect(dataOf(again).valueRevision).toBe(2);
	});

	test("setChecked advances the revision alongside checked and value", () => {
		const updated = applyElementUpdate(
			inputComponent({ type: "checkbox", checked: { literalBool: false } }),
			{ type: "setChecked", checked: true },
		);

		const data = dataOf(updated);
		expect(data.checked).toBe(true);
		expect(data.value).toBe(true);
		expect(data.valueRevision).toBe(1);
	});

	test("updates that do not write a value leave the revision alone", () => {
		const updated = applyElementUpdate(
			inputComponent({ type: "textField", valueRevision: 4 }),
			{ type: "setPlaceholder", placeholder: "Name" },
		);

		expect(dataOf(updated).valueRevision).toBe(4);
	});
});

describe("applyA2UIMessage widget instance routing", () => {
	const widgetHost = (hostId: string, instanceId: string, text: string) =>
		({
			id: hostId,
			component: {
				type: "widgetInstance",
				instanceId,
				widgetId: "shared-widget",
				inlineWidgetDef: {
					rootComponentId: "shared-child",
					components: [
						{
							id: "shared-child",
							component: { type: "text", text },
						},
					],
				},
			},
		}) as unknown as SurfaceComponent;

	const surface = (): Surface => ({
		id: "page-1",
		rootComponentId: "root",
		components: {
			root: {
				id: "root",
				component: { type: "text", text: "Root" },
			} as unknown as SurfaceComponent,
			"host-a": widgetHost("host-a", "instance-a", "First"),
			"host-b": widgetHost("host-b", "instance-b", "Second"),
		},
	});

	const childText = (value: Surface, hostId: string) => {
		const host = value.components[hostId].component as unknown as Record<
			string,
			unknown
		>;
		const inlineDef = host.inlineWidgetDef as {
			components: Array<{ component: Record<string, unknown> }>;
		};
		return inlineDef.components[0].component.text;
	};

	test("routes instanceId/childId to only the matching widget host", () => {
		const updated = applyA2UIMessage(surface(), {
			type: "upsertElement",
			element_id: "instance-b/shared-child",
			value: { type: "setText", text: "Updated" },
		});

		expect(childText(updated, "host-a")).toBe("First");
		expect(childText(updated, "host-b")).toBe("Updated");
	});

	test("preserves slashes inside the widget child id", () => {
		const original = surface();
		const host = original.components["host-b"].component as unknown as Record<
			string,
			unknown
		>;
		const inlineDef = host.inlineWidgetDef as {
			rootComponentId: string;
			components: Array<{ id: string; component: Record<string, unknown> }>;
		};
		inlineDef.rootComponentId = "section/shared-child";
		inlineDef.components[0].id = "section/shared-child";

		const updated = applyA2UIMessage(original, {
			type: "upsertElement",
			element_id: "instance-b/section/shared-child",
			value: { type: "setText", text: "Nested update" },
		});

		expect(childText(updated, "host-a")).toBe("First");
		expect(childText(updated, "host-b")).toBe("Nested update");
	});

	test("retargets a foreign page prefix to this surface's component of the same name", () => {
		const updated = applyA2UIMessage(surface(), {
			type: "upsertElement",
			element_id: "other-page/root",
			value: { type: "setText", text: "Retargeted root" },
		});

		const root = updated.components.root.component as unknown as Record<
			string,
			unknown
		>;
		expect(root.text).toBe("Retargeted root");
		expect(childText(updated, "host-a")).toBe("First");
		expect(childText(updated, "host-b")).toBe("Second");
	});

	test("never retargets a prefix that names a widget instance on this surface", () => {
		const updated = applyA2UIMessage(surface(), {
			type: "upsertElement",
			element_id: "instance-b/root",
			value: { type: "setText", text: "Must not land on the page root" },
		});

		const root = updated.components.root.component as unknown as Record<
			string,
			unknown
		>;
		expect(root.text).not.toBe("Must not land on the page root");
	});

	test("keeps surfaceId/componentId routing for top-level components", () => {
		const updated = applyA2UIMessage(surface(), {
			type: "upsertElement",
			element_id: "page-1/root",
			value: { type: "setText", text: "Updated root" },
		});

		const root = updated.components.root.component as unknown as Record<
			string,
			unknown
		>;
		expect(root.text).toBe("Updated root");
	});

	test("keeps bare child ids as the legacy first-match lookup", () => {
		const updated = applyA2UIMessage(surface(), {
			type: "upsertElement",
			element_id: "shared-child",
			value: { type: "setText", text: "Legacy update" },
		});

		expect(childText(updated, "host-a")).toBe("Legacy update");
		expect(childText(updated, "host-b")).toBe("Second");
	});

	test("does not fall back when the widget instance does not exist", () => {
		const original = surface();
		const updated = applyA2UIMessage(original, {
			type: "upsertElement",
			element_id: "missing-instance/shared-child",
			value: { type: "setText", text: "Wrong target" },
		});

		expect(updated).toBe(original);
	});

	test("queues ordered child updates on a host whose definition is external", () => {
		const original = surface();
		original.components["host-b"] = {
			id: "host-b",
			component: {
				type: "widgetInstance",
				instanceId: "instance-b",
				widgetId: "external-widget",
			},
		} as unknown as SurfaceComponent;

		const withText = applyA2UIMessage(original, {
			type: "upsertElement",
			element_id: "instance-b/shared-child",
			value: { type: "setText", text: "Updated" },
		});
		const updated = applyA2UIMessage(withText, {
			type: "upsertElement",
			element_id: "instance-b/shared-child",
			value: { type: "setVisibility", visible: false },
		});

		const hostA = updated.components["host-a"].component as unknown as Record<
			string,
			unknown
		>;
		const hostB = updated.components["host-b"].component as unknown as Record<
			string,
			unknown
		>;
		expect(hostA.runtimeChildUpdates).toBeUndefined();
		expect(hostB.runtimeChildUpdates).toEqual({
			"shared-child": [
				{ type: "setText", text: "Updated" },
				{ type: "setVisibility", visible: false },
			],
		});
	});
});
