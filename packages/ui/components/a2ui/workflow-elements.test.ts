import { describe, expect, test } from "bun:test";
import type { SurfaceComponent } from "./types";
import {
	type WidgetElementComponent,
	collectEventRelevantInputValues,
	elementValueScopeIds,
	flattenSurfaceComponentsForElements,
	legacyWidgetValueSurfaceId,
	mergeStoredElementValues,
} from "./workflow-elements";

function component(
	id: string,
	data: Record<string, unknown>,
	eventRelevant = false,
): SurfaceComponent {
	return { id, component: data, eventRelevant } as unknown as SurfaceComponent;
}

function widgetHost(
	hostId: string,
	instanceId: string,
	children?: WidgetElementComponent[],
): SurfaceComponent {
	return component(hostId, {
		type: "widgetInstance",
		instanceId,
		widgetId: "shared-widget",
		...(children
			? {
					inlineWidgetDef: {
						rootComponentId: children[0]?.id,
						components: children,
					},
				}
			: {}),
	});
}

const field = (
	id: string,
	value: string,
	eventRelevant = true,
): WidgetElementComponent => ({
	id,
	eventRelevant,
	component: { type: "textField", value: { literalString: value } },
});

describe("widget-scoped workflow elements", () => {
	test("keeps instance keys available to ordinary page callbacks", () => {
		const components = {
			host: widgetHost("host", "instance", [field("field", "initial")]),
		};
		const flattened = flattenSurfaceComponentsForElements(components, "page-1");
		expect(flattened["instance/field"]).toBeDefined();
		expect(flattened["page-1/field"]).toBeDefined();

		const merged = mergeStoredElementValues(
			{},
			{ "instance/field": "current" },
			components,
			"page-1",
		);
		const child = merged["instance/field"] as Record<string, unknown>;
		expect((child.component as Record<string, unknown>).value).toEqual({
			literalString: "current",
		});
		const legacyChild = merged["page-1/field"] as Record<string, unknown>;
		expect((legacyChild.component as Record<string, unknown>).value).toEqual({
			literalString: "current",
		});
	});

	test("includes only the triggering widget and keys its resolved children by instance", () => {
		const components = {
			root: component("root", { type: "column" }),
			"host-a": widgetHost("host-a", "instance-a", [
				field("shared-field", "A"),
			]),
			"host-b": widgetHost("host-b", "instance-b"),
		};
		const scope = {
			instanceId: "instance-b",
			components: [field("shared-field", "B")],
		};

		const flattened = flattenSurfaceComponentsForElements(
			components,
			"page-1",
			scope,
		);

		expect(Object.keys(flattened).sort()).toEqual([
			"instance-b/shared-field",
			"page-1/host-b",
			"page-1/root",
		]);
		expect(
			(flattened["instance-b/shared-field"] as Record<string, unknown>).id,
		).toBe("shared-field");
	});

	test("merges only the triggering instance's values and drops other widget snapshots", () => {
		const components = {
			root: component("root", { type: "column" }),
			"host-a": widgetHost("host-a", "instance-a", [
				field("shared-field", "A"),
			]),
			"host-b": widgetHost("host-b", "instance-b", [
				field("shared-field", "B"),
			]),
		};
		const scope = {
			instanceId: "instance-b",
			components: [field("shared-field", "B")],
		};
		const backendElements = {
			"page-1/host-a": components["host-a"],
			"instance-a/shared-field": field("shared-field", "stale A"),
		};

		const merged = mergeStoredElementValues(
			backendElements,
			{
				"instance-a/shared-field": "edited A",
				"instance-b/shared-field": "edited B",
				"micro-a/values": { selected: "A" },
			},
			components,
			"page-1",
			scope,
		);

		expect(merged["page-1/host-a"]).toBeUndefined();
		expect(merged["instance-a/shared-field"]).toBeUndefined();
		expect(merged["micro-a/values"]).toBeUndefined();
		const child = merged["instance-b/shared-field"] as Record<string, unknown>;
		expect((child.component as Record<string, unknown>).value).toEqual({
			literalString: "edited B",
		});
	});

	test("does not reuse an ambiguous legacy page value across repeated widgets", () => {
		const components = {
			"host-a": widgetHost("host-a", "instance-a", [field("field", "A")]),
			"host-b": widgetHost("host-b", "instance-b", [field("field", "B")]),
		};
		const scope = {
			instanceId: "instance-b",
			components: [field("field", "B")],
		};

		expect(
			legacyWidgetValueSurfaceId(components, "page-1", scope),
		).toBeUndefined();
		const merged = mergeStoredElementValues(
			{},
			{ "page-1/field": "ambiguous" },
			components,
			"page-1",
			scope,
		);
		const child = merged["instance-b/field"] as Record<string, unknown>;
		expect((child.component as Record<string, unknown>).value).toEqual({
			literalString: "B",
		});
	});

	test("retains the legacy value migration for a single widget instance", () => {
		const components = {
			host: widgetHost("host", "instance", [field("field", "initial")]),
		};
		const scope = {
			instanceId: "instance",
			components: [field("field", "initial")],
		};
		const merged = mergeStoredElementValues(
			{},
			{ "page-1/field": "restored" },
			components,
			"page-1",
			scope,
		);
		const child = merged["instance/field"] as Record<string, unknown>;
		expect((child.component as Record<string, unknown>).value).toEqual({
			literalString: "restored",
		});
	});

	test("scopes event-relevant inputs and persisted-value restoration", () => {
		const components = {
			host: widgetHost("host", "instance", [field("field", "initial")]),
			micro: component("micro", {
				type: "microWidgetInstance",
				instanceId: "micro-instance",
			}),
		};

		expect(elementValueScopeIds(components, "page-1")).toEqual([
			"page-1",
			"instance",
			"micro-instance",
		]);
		expect(
			collectEventRelevantInputValues(
				{
					"instance/field": "current",
					"other/field": "wrong",
				},
				[field("field", "initial"), field("ignored", "", false)],
				"instance",
			),
		).toEqual({ field: "current" });
	});

	test("includes only the active micro widget's mirrored values", () => {
		const components = {
			"micro-a": component("micro-a", {
				type: "microWidgetInstance",
				instanceId: "instance-a",
			}),
			"micro-b": component("micro-b", {
				type: "microWidgetInstance",
				instanceId: "instance-b",
			}),
		};
		const merged = mergeStoredElementValues(
			{},
			{
				"instance-a/values": { choice: "A" },
				"instance-b/values": { choice: "B" },
			},
			components,
			"page-1",
			{ instanceId: "instance-b" },
		);

		expect(merged["instance-a/values"]).toBeUndefined();
		expect(merged["instance-b/values"]).toBeDefined();
		expect(merged["page-1/micro-a"]).toBeUndefined();
		expect(merged["page-1/micro-b"]).toBeDefined();
	});
});
