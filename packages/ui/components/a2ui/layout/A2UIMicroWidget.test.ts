import { describe, expect, test } from "bun:test";
import type { Action, MicroWidgetInstanceComponent } from "../types";
import { resolveMicroWidgetEventRoute } from "./A2UIMicroWidget";

const action = (name: string): Action => ({ name, context: {} });

const component = (
	overrides: Partial<MicroWidgetInstanceComponent> = {},
): MicroWidgetInstanceComponent => ({
	id: "sales-chart",
	type: "microWidgetInstance",
	instanceId: "sales-chart",
	packageId: "com.example.sales",
	widgetId: "chart",
	packageVersion: "1.0.0",
	...overrides,
});

describe("resolveMicroWidgetEventRoute", () => {
	test("prefers exact named handlers and preserves their order", () => {
		const route = resolveMicroWidgetEventRoute(
			component({
				eventHandlers: {
					pointSelected: [action("workflow_event"), action("navigate_page")],
				},
				actionBindings: { pointSelected: { workflow: {} } },
				actions: [action("legacy")],
			}),
			"pointSelected",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("workflow_event"), action("navigate_page")],
		});
	});

	test("uses wildcard handlers before widget bindings", () => {
		const route = resolveMicroWidgetEventRoute(
			component({
				eventHandlers: { "*": [action("external_link")] },
				actionBindings: { refreshRequested: { workflow: {} } },
			}),
			"refreshRequested",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("external_link")],
		});
	});

	test("an explicit empty handler suppresses every fallback", () => {
		const route = resolveMicroWidgetEventRoute(
			component({
				eventHandlers: {
					pointSelected: [],
					"*": [action("wildcard")],
				},
				actionBindings: { pointSelected: { workflow: {} } },
				actions: [action("legacy")],
			}),
			"pointSelected",
		);

		expect(route).toEqual({ kind: "actions", actions: [] });
	});

	test("keeps an existing action binding on the widget_event path", () => {
		expect(
			resolveMicroWidgetEventRoute(
				component({
					actionBindings: { pointSelected: { workflow: {} } },
					actions: [action("legacy")],
				}),
				"pointSelected",
			),
		).toEqual({ kind: "widget_event" });
	});

	test("falls back to only actions[0] when no binding exists", () => {
		const route = resolveMicroWidgetEventRoute(
			component({ actions: [action("legacy"), action("previously-inert")] }),
			"pointSelected",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("legacy")],
		});
	});

	test("preserves widget_event diagnostics when nothing is configured", () => {
		expect(resolveMicroWidgetEventRoute(component(), "pointSelected")).toEqual({
			kind: "widget_event",
		});
	});
});
