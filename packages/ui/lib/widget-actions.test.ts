import { describe, expect, test } from "bun:test";
import type { SurfaceComponent } from "../components/a2ui/types";
import {
	checkWidgetActionId,
	declareWidgetActions,
	findUndeclaredWidgetActions,
	normalizeWidgetActionId,
	renameWidgetActionInComponents,
	resolveWidgetEventTarget,
} from "./widget-actions";

function component(
	id: string,
	data: Record<string, unknown>,
): SurfaceComponent {
	return { id, component: { type: "button", ...data } } as SurfaceComponent;
}

describe("widget action ids", () => {
	test("normalizes whitespace into a usable id", () => {
		expect(normalizeWidgetActionId("  toggle feed  ")).toBe("toggle-feed");
	});

	test("rejects empty, malformed and taken ids", () => {
		expect(checkWidgetActionId("", [])).toBe("empty");
		expect(checkWidgetActionId("-leading", [])).toBe("invalid");
		expect(checkWidgetActionId("has/slash", [])).toBe("invalid");
		expect(checkWidgetActionId("edit-feed", ["edit-feed"])).toBe("duplicate");
		expect(checkWidgetActionId("edit-feed", ["test-feed"])).toBeNull();
	});
});

describe("renameWidgetActionInComponents", () => {
	test("rewrites legacy actions and event handlers", () => {
		const components = [
			component("a", {
				actions: [{ name: "widget_event", context: { actionId: "old" } }],
			}),
			component("b", {
				eventHandlers: {
					onClick: [
						{ name: "widget_event", context: { actionId: "old" } },
						{ name: "navigate_page", context: { route: "/x" } },
					],
					onLongPress: [
						{ name: "widget_event", context: { actionId: "other" } },
					],
				},
			}),
		];

		const [first, second] = renameWidgetActionInComponents(
			components,
			"old",
			"new",
		).map((entry) => entry.component);

		expect(first.actions?.[0].context.actionId).toBe("new");
		expect(second.eventHandlers?.onClick[0].context.actionId).toBe("new");
		expect(second.eventHandlers?.onClick[1]).toEqual({
			name: "navigate_page",
			context: { route: "/x" },
		});
		expect(second.eventHandlers?.onLongPress[0].context.actionId).toBe("other");
	});

	test("returns the original array when nothing references the id", () => {
		const components = [component("a", { eventHandlers: {} })];
		expect(renameWidgetActionInComponents(components, "old", "new")).toBe(
			components,
		);
	});
});

describe("findUndeclaredWidgetActions", () => {
	test("reports ids fired by components but missing from the declarations", () => {
		const components = [
			component("a", {
				actions: [{ name: "widget_event", context: { actionId: "ghost" } }],
			}),
			component("b", {
				eventHandlers: {
					onClick: [
						{ name: "widget_event", context: { actionId: "ghost" } },
						{ name: "widget_event", context: { actionId: "declared" } },
					],
					onLongPress: [
						{ name: "widget_event", context: { actionId: "ghost" } },
						{ name: "widget_event", context: { actionId: "phantom" } },
					],
				},
			}),
		];

		expect(
			findUndeclaredWidgetActions(components, [{ id: "declared" }]),
		).toEqual([
			{ actionId: "ghost", componentIds: ["a", "b"] },
			{ actionId: "phantom", componentIds: ["b"] },
		]);
	});

	test("ignores other actions and widget events without an id", () => {
		const components = [
			component("a", {
				actions: [
					{ name: "workflow_event", context: { actionId: "not-a-widget-id" } },
					{ name: "widget_event", context: { actionId: "" } },
					{ name: "widget_event" },
				],
			}),
		];
		expect(findUndeclaredWidgetActions(components, undefined)).toEqual([]);
	});
});

describe("declareWidgetActions", () => {
	test("appends one declaration per new id and keeps existing ones", () => {
		const existing = [{ id: "open", label: "Open", contextSchema: [] }];
		expect(declareWidgetActions(existing, ["open", "close", "close"])).toEqual([
			{ id: "open", label: "Open", contextSchema: [] },
			{ id: "close", label: "close", contextSchema: [] },
		]);
	});
});

describe("resolveWidgetEventTarget", () => {
	const declared = [{ id: "open", label: "Open" }];

	test("finds the declaration of a declared widget event", () => {
		expect(
			resolveWidgetEventTarget(
				{ name: "widget_event", context: { actionId: "open" } },
				declared,
			),
		).toEqual({
			actionId: "open",
			declaration: declared[0],
			undeclared: undefined,
		});
	});

	test("reports an id the widget does not declare", () => {
		expect(
			resolveWidgetEventTarget(
				{ name: "widget_event", context: { actionId: "ghost" } },
				declared,
			).undeclared,
		).toBe("ghost");
	});

	test("cannot judge without declarations or without a widget event", () => {
		const ghost = { name: "widget_event", context: { actionId: "ghost" } };
		expect(
			resolveWidgetEventTarget(ghost, undefined).undeclared,
		).toBeUndefined();
		expect(
			resolveWidgetEventTarget(
				{ name: "workflow_event", context: { actionId: "ghost" } },
				declared,
			),
		).toEqual({ actionId: "", declaration: undefined, undeclared: undefined });
	});
});
