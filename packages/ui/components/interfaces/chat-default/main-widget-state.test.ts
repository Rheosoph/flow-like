import { describe, expect, test } from "bun:test";
import type { IChatMainWidget, IMessage } from "./chat-db";
import {
	refreshMainChatWidget,
	resolveMainChatWidget,
} from "./main-widget-state";

const main = {
	widget: {
		instance_id: "browser",
		widget_id: "browser-widget",
		surface_id: "browser",
		component: { type: "widgetInstance" },
	},
	title: "Browser",
} satisfies IChatMainWidget;
const message = (
	id: string,
	sessionId: string,
	main_widget?: IChatMainWidget,
): IMessage => ({
	id,
	sessionId,
	appId: "app",
	timestamp: 0,
	files: [],
	inner: { role: "assistant", content: "" } as IMessage["inner"],
	main_widget,
});

describe("refreshMainChatWidget", () => {
	const initial = {
		...main,
		command_id: "open",
		command_at: 100,
		updated_at: 100,
	};

	test("adopts newer feedback only for the same command and widget", () => {
		const target = message("run", "session", initial);
		const latest = { ...initial, updated_at: 200 };
		refreshMainChatWidget(target, latest);
		expect(target.main_widget).toBe(latest);
		refreshMainChatWidget(target, initial);
		expect(target.main_widget).toBe(latest);
	});

	test("does not adopt a clear or a different command into an older run", () => {
		const target = message("run", "session", initial);
		for (const latest of [
			{ ...initial, widget: null, command_id: "clear", updated_at: 200 },
			{ ...initial, command_id: "replacement", updated_at: 200 },
			{
				...initial,
				widget: { ...main.widget, instance_id: "other" },
				updated_at: 200,
			},
		]) {
			refreshMainChatWidget(target, latest);
			expect(target.main_widget).toBe(initial);
		}
	});
});

describe("resolveMainChatWidget", () => {
	test("preserves the latest pane across subsequent text-only turns", () => {
		expect(
			resolveMainChatWidget(
				[message("one", "session", main), message("two", "session")],
				"session",
			),
		).toBe(main);
	});

	test("a clear command stops resolution instead of resurrecting an older pane", () => {
		const cleared = { widget: null };
		expect(
			resolveMainChatWidget(
				[
					message("one", "session", main),
					message("two", "session", cleared),
					message("three", "session"),
				],
				"session",
			),
		).toBe(cleared);
	});

	test("uses the live turn and rejects history from another session", () => {
		const history = [message("one", "other", main)];
		expect(resolveMainChatWidget(history, "session")).toBeUndefined();
		expect(
			resolveMainChatWidget(
				history,
				"session",
				message("live", "session", main),
			),
		).toBe(main);
		expect(
			resolveMainChatWidget(history, "session", message("live", "other", main)),
		).toBeUndefined();
	});

	test("a live clear overrides saved history", () => {
		const cleared = { widget: null };
		expect(
			resolveMainChatWidget(
				[message("one", "session", main)],
				"session",
				message("live", "session", cleared),
			),
		).toBe(cleared);
	});

	test("a later text turn cannot mask a newer widget change in an earlier concurrent run", () => {
		const updated = { ...main, title: "Updated browser", updated_at: 200 };
		const inherited = { ...main, updated_at: 100 };
		const first = { ...message("first", "session", updated), timestamp: 10 };
		const second = {
			...message("second", "session", inherited),
			timestamp: 20,
		};
		expect(resolveMainChatWidget([first, second], "session")).toBe(updated);
		expect(resolveMainChatWidget([first], "session", second)).toBe(updated);
	});

	test("late updates to an exchanged widget cannot supersede the replacement", () => {
		const oldWidget = { ...main, command_at: 100, updated_at: 300 };
		const replacement = {
			...main,
			title: "Library",
			command_at: 200,
			updated_at: 200,
		};
		expect(
			resolveMainChatWidget(
				[message("new", "session", replacement)],
				"session",
				message("old", "session", oldWidget),
			),
		).toBe(replacement);
	});

	test("uses message timestamps for old history without a frontend revision", () => {
		const first = { ...message("first", "session", main), timestamp: 10 };
		const cleared = { widget: null };
		const second = { ...message("second", "session", cleared), timestamp: 20 };
		expect(resolveMainChatWidget([second, first], "session")).toBe(cleared);
	});
});
