import { describe, expect, test } from "bun:test";
import {
	foldA2UIServerMessage,
	resolveElementUpdateSurfaceId,
} from "./fold-surfaces";
import type { Surface, SurfaceComponent } from "./types";

function component(
	id: string,
	data: Record<string, unknown>,
): SurfaceComponent {
	return { id, component: data } as unknown as SurfaceComponent;
}

function surface(id: string, components: SurfaceComponent[]): Surface {
	return {
		id,
		rootComponentId: components[0]?.id ?? "root",
		components: Object.fromEntries(components.map((item) => [item.id, item])),
	};
}

function widgetHost(
	hostId: string,
	instanceId: string,
	childId = "field",
): SurfaceComponent {
	return component(hostId, {
		type: "widgetInstance",
		instanceId,
		widgetId: "form-widget",
		inlineWidgetDef: {
			name: "Form",
			rootComponentId: childId,
			components: [
				{
					id: childId,
					component: {
						type: "textField",
						value: { literalString: "before" },
					},
				},
			],
		},
	});
}

describe("resolveElementUpdateSurfaceId", () => {
	test("resolves widget and micro-widget instance prefixes to their owner", () => {
		const surfaces = new Map([
			["page-a", surface("page-a", [widgetHost("host-a", "widget-a")])],
			[
				"page-b",
				surface("page-b", [
					component("micro-host", {
						type: "microWidgetInstance",
						instanceId: "micro-b",
					}),
				]),
			],
		]);

		expect(resolveElementUpdateSurfaceId(surfaces, "widget-a/field")).toBe(
			"page-a",
		);
		expect(resolveElementUpdateSurfaceId(surfaces, "micro-b/values")).toBe(
			"page-b",
		);
	});

	test("preserves surface prefixes and the unscoped first-surface fallback", () => {
		const surfaces = new Map([
			["page-a", surface("page-a", [widgetHost("host-a", "page-b")])],
			["page-b", surface("page-b", [component("field", { type: "text" })])],
		]);

		expect(resolveElementUpdateSurfaceId(surfaces, "page-b/field")).toBe(
			"page-b",
		);
		expect(resolveElementUpdateSurfaceId(surfaces, "field")).toBe("page-a");
		expect(resolveElementUpdateSurfaceId(surfaces, "missing/field")).toBe(
			undefined,
		);
	});
});

describe("foldA2UIServerMessage", () => {
	test("routes an instance-scoped update to that widget's surface", () => {
		const pageA = surface("page-a", [widgetHost("host-a", "widget-a")]);
		const pageB = surface("page-b", [widgetHost("host-b", "widget-b")]);
		const surfaces = new Map([
			[pageA.id, pageA],
			[pageB.id, pageB],
		]);

		const next = foldA2UIServerMessage(surfaces, {
			type: "upsertElement",
			element_id: "widget-b/field",
			value: { type: "setValue", value: "after" },
		});

		expect(next.get("page-a")).toBe(pageA);
		const host = next.get("page-b")?.components["host-b"]
			.component as unknown as Record<string, unknown>;
		const inlineDef = host.inlineWidgetDef as {
			components: Array<{ component: Record<string, unknown> }>;
		};
		expect(inlineDef.components[0]?.component.value).toEqual({
			literalString: "after",
		});
	});
});
