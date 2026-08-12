import { describe, expect, test } from "bun:test";
import type { Action, ActionBinding } from "../types";
import {
	type WidgetInstanceContextValue,
	resolveWidgetInstanceEventRoute,
} from "./A2UIWidgetInstance";

const action = (name: string): Action => ({ name, context: {} });

const workflowBinding: ActionBinding = {
	workflow: { flowId: "flow-id", inputMappings: {} },
};

const widgetInstance = (
	overrides: Partial<WidgetInstanceContextValue> = {},
): WidgetInstanceContextValue => ({
	instanceId: "instance-id",
	widgetId: "widget-id",
	actionBindings: {},
	...overrides,
});

describe("resolveWidgetInstanceEventRoute", () => {
	test("prefers exact named handlers and preserves their order", () => {
		const route = resolveWidgetInstanceEventRoute(
			widgetInstance({
				eventHandlers: {
					submit: [action("workflow_event"), action("navigate_page")],
					"*": [action("wildcard")],
				},
				actionBindings: { submit: workflowBinding },
				actions: [action("legacy")],
			}),
			"submit",
		);

		expect(route).toEqual({
			kind: "actions",
			actions: [action("workflow_event"), action("navigate_page")],
		});
	});

	test("uses wildcard handlers before classic bindings", () => {
		expect(
			resolveWidgetInstanceEventRoute(
				widgetInstance({
					eventHandlers: { "*": [action("external_link")] },
					actionBindings: { refresh: workflowBinding },
				}),
				"refresh",
			),
		).toEqual({ kind: "actions", actions: [action("external_link")] });
	});

	test("treats an explicit empty named handler as handled", () => {
		expect(
			resolveWidgetInstanceEventRoute(
				widgetInstance({
					eventHandlers: {
						submit: [],
						"*": [action("wildcard")],
					},
					actionBindings: { submit: workflowBinding },
					actions: [action("legacy")],
				}),
				"submit",
			),
		).toEqual({ kind: "actions", actions: [] });
	});

	test("keeps a classic binding ahead of the legacy component action", () => {
		expect(
			resolveWidgetInstanceEventRoute(
				widgetInstance({
					actionBindings: { submit: workflowBinding },
					actions: [action("legacy")],
				}),
				"submit",
			),
		).toEqual({ kind: "binding", binding: workflowBinding });
	});

	test("falls back to only the first legacy component action", () => {
		expect(
			resolveWidgetInstanceEventRoute(
				widgetInstance({
					actions: [action("legacy"), action("previously-inert")],
				}),
				"submit",
			),
		).toEqual({ kind: "actions", actions: [action("legacy")] });
	});

	test("preserves the no-binding diagnostic route", () => {
		expect(resolveWidgetInstanceEventRoute(widgetInstance(), "submit")).toEqual(
			{ kind: "diagnostic" },
		);
	});
});
