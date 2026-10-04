import { describe, expect, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import type { AppPackageWidget } from "../../lib/package-widgets";
import type { IPage, IWidgetRef } from "../../state/backend-state/page-state";
import {
	reloadMicroWidgetsInComponents,
	reloadMicroWidgetsInPage,
} from "./micro-widget-project-reload";
import type { MicroWidgetInstanceComponent, SurfaceComponent } from "./types";

const OLD_HASH = "a".repeat(64);
const NEW_HASH = "b".repeat(64);

function contract(
	inputs: WidgetContract["inputs"] = {},
	events: WidgetContract["events"] = {},
): WidgetContract {
	return { contractVersion: 1, id: "map", inputs, events, queries: {} };
}

function installed(
	packageId: string,
	next = contract(),
	overrides: Partial<AppPackageWidget> = {},
): AppPackageWidget {
	return {
		packageId,
		packageName: packageId,
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
	id: string,
	packageId = "geo",
	overrides: Partial<MicroWidgetInstanceComponent> = {},
): SurfaceComponent & { component: MicroWidgetInstanceComponent } {
	return {
		id,
		style: { className: "surface" },
		eventRelevant: true,
		component: {
			id,
			type: "microWidgetInstance",
			instanceId: `instance-${id}`,
			packageId,
			widgetId: "map",
			packageVersion: "1.0.0",
			bundleHash: OLD_HASH,
			contract: contract(),
			props: {},
			...overrides,
		},
	};
}

function widgetRef(components: SurfaceComponent[]): IWidgetRef {
	return {
		id: "widget",
		name: "Widget",
		rootComponentId: components[0]?.id ?? "root",
		components,
		tags: [],
		dataModel: [{ path: "title", value: "Configured data" }],
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-01T00:00:00Z",
	};
}

function page(overrides: Partial<IPage> = {}): IPage {
	return {
		id: "page",
		name: "Page",
		boardId: "board",
		route: "/map",
		content: [],
		components: [],
		layoutType: "freeform",
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-01T00:00:00Z",
		...overrides,
	};
}

describe("reloadMicroWidgetsInComponents", () => {
	test("updates every installed package with a stale hash or version", () => {
		const local = placed("local", "local-package");
		const privateWidget = placed("private", "private-package");
		const publicWidget = placed("public", "public-package");
		const current = placed("current", "local-package", {
			bundleHash: NEW_HASH,
		});
		const unavailable = placed("unavailable", "missing-package");
		const otherWidget = placed("other-widget", "local-package", {
			widgetId: "chart",
		});
		const text: SurfaceComponent = {
			id: "text",
			component: {
				id: "text",
				type: "text",
				content: { literalString: "Map" },
			},
		};
		const components = [
			local,
			privateWidget,
			publicWidget,
			current,
			unavailable,
			otherWidget,
			text,
		];
		const result = reloadMicroWidgetsInComponents(components, [
			installed("local-package"),
			installed("private-package", contract(), {
				packageVersion: "1.1.0",
				bundleHash: OLD_HASH,
			}),
			installed("public-package", contract(), {
				packageVersion: "2.0.0",
				bundleHash: undefined,
			}),
		]);

		expect(result.count).toBe(3);
		expect(result.unavailable).toEqual(["local-package/chart"]);
		expect(result.components[0].component).toMatchObject({
			bundleHash: NEW_HASH,
			packageVersion: "1.0.0",
		});
		expect(result.components[1].component).toMatchObject({
			bundleHash: OLD_HASH,
			packageVersion: "1.1.0",
		});
		expect(result.components[2].component).toMatchObject({
			bundleHash: OLD_HASH,
			packageVersion: "2.0.0",
		});
		for (let index = 3; index < components.length; index += 1) {
			expect(result.components[index]).toBe(components[index]);
		}
		expect(local.component.bundleHash).toBe(OLD_HASH);
		expect(privateWidget.component.packageVersion).toBe("1.0.0");
		expect(publicWidget.component.packageVersion).toBe("1.0.0");
	});

	test("preserves component identity, routes, styles, and accepted data", () => {
		const config = { color: "red" };
		const previous = contract({ config: { type: "json" } }, { selected: {} });
		const original = placed("map", "geo", {
			contract: previous,
			props: { config },
			eventHandlers: { selected: [{ name: "workflow_event", context: {} }] },
			actionBindings: { selected: { nodeId: "evt-1" } },
			actions: [{ name: "workflow_event", context: { nodeId: "evt-2" } }],
			style: { className: "h-64" },
		});
		const snapshot = structuredClone(original);
		const result = reloadMicroWidgetsInComponents(
			[original],
			[installed("geo", previous)],
		);
		const updated = result.components[0];
		expect(updated).not.toBe(original);
		expect(updated).toMatchObject({
			id: original.id,
			eventRelevant: true,
			component: { id: original.id, instanceId: "instance-map" },
		});
		expect(updated.style).toBe(original.style);
		expect(updated.component.style).toBe(original.component.style);
		expect(updated.component.eventHandlers).toBe(
			original.component.eventHandlers,
		);
		expect(updated.component.actions).toBe(original.component.actions);
		const updatedMicroWidget =
			updated.component as MicroWidgetInstanceComponent;
		expect(updatedMicroWidget.actionBindings).toBe(
			original.component.actionBindings,
		);
		expect(updatedMicroWidget.props?.config).toBe(config);
		expect(updatedMicroWidget.contract).not.toBe(previous);
		expect(original).toEqual(snapshot);
	});
});

describe("reloadMicroWidgetsInPage", () => {
	test("reports missing widget definitions from every store without changing those instances", () => {
		const missing = placed("missing", "geo", { widgetId: "removed" });
		const original = page({
			components: [placed("available"), missing],
			content: [
				{ Component: placed("inline", "geo", { widgetId: "removed" }) },
			],
			widgetRefs: {
				nested: widgetRef([
					placed("embedded", "geo", { widgetId: "removed-embedded" }),
					placed("unrelated", "uninstalled-package"),
				]),
			},
		});

		const result = reloadMicroWidgetsInPage(original, [installed("geo")]);

		expect(result.count).toBe(1);
		expect(result.unavailable).toEqual(["geo/removed", "geo/removed-embedded"]);
		expect(result.page.components[1]).toBe(missing);
		expect(result.page.content).toBe(original.content);
		expect(result.page.widgetRefs).toBe(original.widgetRefs);
	});

	test("updates page components, inline content, and embedded widget definitions", () => {
		const inline = placed("inline");
		const unchangedContent = { ComponentRef: "root" };
		const changedRef = widgetRef([placed("nested")]);
		const currentRef = widgetRef([
			placed("current", "geo", { bundleHash: NEW_HASH }),
		]);
		const original = page({
			components: [placed("root")],
			content: [unchangedContent, { Component: inline }],
			widgetRefs: { nested: changedRef, current: currentRef },
			canvasSettings: { backgroundColor: "#ffffff" },
		});
		const snapshot = structuredClone(original);
		const result = reloadMicroWidgetsInPage(original, [installed("geo")]);

		expect(result.count).toBe(3);
		expect(result.page.components[0].component).toMatchObject({
			bundleHash: NEW_HASH,
		});
		expect(result.page.content[1]).toMatchObject({
			Component: { component: { bundleHash: NEW_HASH } },
		});
		expect(
			result.page.widgetRefs?.nested.components[0].component,
		).toMatchObject({
			bundleHash: NEW_HASH,
		});
		expect(result.page.content[0]).toBe(unchangedContent);
		expect(result.page.widgetRefs?.current).toBe(currentRef);
		expect(result.page.widgetRefs?.nested.dataModel).toBe(changedRef.dataModel);
		expect(result.page.canvasSettings).toBe(original.canvasSettings);
		expect(result.page.id).toBe(original.id);
		expect(result.page.boardId).toBe(original.boardId);
		expect(result.page.route).toBe(original.route);
		expect(original).toEqual(snapshot);
	});

	test("migrates contracts everywhere and reports each affected input or event once", () => {
		const previous = contract(
			{
				count: { type: "string" },
				zoom: { type: "number", default: 4 },
				mode: { type: "string" },
				legacy: { type: "string" },
			},
			{ moved: {} },
		);
		const next = contract({
			count: { type: "integer" },
			zoom: { type: "number", default: 6 },
			mode: { type: "enum", choices: ["light", "dark"], default: "light" },
			units: { type: "string", default: "km" },
		});
		const makeInstance = (id: string) =>
			placed(id, "geo", {
				contract: previous,
				props: { count: "12", zoom: 4, mode: "sepia", legacy: "old" },
				eventHandlers: { moved: [{ name: "workflow_event", context: {} }] },
			});
		const original = page({
			components: [makeInstance("root")],
			content: [{ Component: makeInstance("inline") }],
			widgetRefs: { nested: widgetRef([makeInstance("nested")]) },
		});
		const result = reloadMicroWidgetsInPage(original, [installed("geo", next)]);

		expect(result.count).toBe(3);
		expect(result.report).toEqual({
			converted: ["count"],
			defaulted: ["zoom", "units"],
			reset: ["mode"],
			removed: ["legacy"],
			undeclaredEvents: ["moved"],
		});
		const inlineContent = result.page.content[0];
		if (!("Component" in inlineContent))
			throw new Error("Missing inline component");
		const nested = result.page.widgetRefs?.nested.components[0];
		if (!nested) throw new Error("Missing embedded widget component");
		const updatedInstances = [
			result.page.components[0],
			inlineContent.Component,
			nested,
		];
		for (const surface of updatedInstances) {
			expect(surface.component).toMatchObject({
				contract: next,
				props: { count: 12, zoom: 6, mode: "light", units: "km" },
				eventHandlers: { moved: [{ name: "workflow_event" }] },
			});
		}
	});

	test("keeps unchanged pages and component stores by reference", () => {
		const original = page({
			components: [placed("current", "geo", { bundleHash: NEW_HASH })],
			content: [{ ComponentRef: "current" }],
			widgetRefs: { unavailable: widgetRef([placed("missing", "missing")]) },
		});
		for (const available of [[installed("geo")], undefined]) {
			const result = reloadMicroWidgetsInPage(original, available);
			expect(result.page).toBe(original);
			expect(result.count).toBe(0);
			expect(
				Object.values(result.report).every((keys) => keys.length === 0),
			).toBe(true);
			expect(
				reloadMicroWidgetsInComponents(original.components, available)
					.components,
			).toBe(original.components);
		}
	});

	test("keeps unrelated stores by reference when only an embedded widget changes", () => {
		const original = page({
			content: [{ ComponentRef: "root" }],
			widgetRefs: { nested: widgetRef([placed("nested")]) },
		});
		const result = reloadMicroWidgetsInPage(original, [installed("geo")]);
		expect(result.count).toBe(1);
		expect(result.page.components).toBe(original.components);
		expect(result.page.content).toBe(original.content);
		expect(result.page.widgetRefs).not.toBe(original.widgetRefs);
	});
});
