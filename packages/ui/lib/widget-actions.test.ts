import { describe, expect, test } from "bun:test";
import type { SurfaceComponent } from "../components/a2ui/types";
import {
	checkWidgetActionId,
	normalizeWidgetActionId,
	renameWidgetActionInComponents,
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

		const next = renameWidgetActionInComponents(components, "old", "new");
		const first = next[0].component as {
			actions: { context: { actionId: string } }[];
		};
		const second = next[1].component as {
			eventHandlers: Record<
				string,
				{ name: string; context: Record<string, unknown> }[]
			>;
		};

		expect(first.actions[0].context.actionId).toBe("new");
		expect(second.eventHandlers.onClick[0].context.actionId).toBe("new");
		expect(second.eventHandlers.onClick[1]).toEqual({
			name: "navigate_page",
			context: { route: "/x" },
		});
		expect(second.eventHandlers.onLongPress[0].context.actionId).toBe("other");
	});

	test("returns the original array when nothing references the id", () => {
		const components = [component("a", { eventHandlers: {} })];
		expect(renameWidgetActionInComponents(components, "old", "new")).toBe(
			components,
		);
	});
});
