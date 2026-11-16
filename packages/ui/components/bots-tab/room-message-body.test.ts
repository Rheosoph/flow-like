import { expect, test } from "bun:test";
import type { ConversationMessage } from "./fold-entries";
import { roomChatMessage } from "./room-message-body";

test("room messages retain the sending bot's file context outside the response text", () => {
	const text = "![Generated image](bot/images/img-1.png)";
	const message: ConversationMessage = {
		type: "message",
		id: "image-reply",
		senderId: "bot_scout",
		mine: false,
		createdAt: "2026-11-08T12:00:00Z",
		time: 1,
		content: { kind: "text", text },
		usageStats: [
			{
				step_name: "Assistant",
				stats: {
					usage: {
						prompt_tokens: 100,
						completion_tokens: 10,
						total_tokens: 110,
						prompt_tokens_details: {
							cached_tokens: 80,
							cache_write_tokens: 10,
						},
					},
					context: {
						estimated_tokens: 100,
						context_window: 1_000,
						reserved_output_tokens: 100,
					},
					compactions: [
						{
							kind: "turn",
							trigger: "soft",
							status: "completed",
							before_tokens: 300,
							after_tokens: 100,
							duration_ms: 10,
						},
					],
				},
			},
		],
		mentions: [],
		edited: false,
		reactions: [],
	};
	const rendered = roomChatMessage(message, "room", {
		storageBotId: message.senderId,
	});
	expect(rendered.storageBotId).toBe("bot_scout");
	expect(rendered.source_text).toBe(text);
	expect(rendered.inner.content).toBe(text);
	expect(rendered.usage_stats).toEqual(message.usageStats);
	expect(roomChatMessage(message, "room").storageBotId).toBeUndefined();
});

test("room messages show the app widgets a bot put on its reply", () => {
	const widget = {
		instance_id: "w1",
		widget_id: "chart",
		surface_id: "s1",
		component: { widgetInstance: {} },
	};
	const handle = {
		app_id: "conversation_room1",
		path: "f1.json",
		bytes: 50_000,
		sha256: "abc",
		mime: "application/json",
	};
	const message: ConversationMessage = {
		type: "message",
		id: "widget-reply",
		senderId: "bot_scout",
		mine: false,
		createdAt: "2026-11-16T12:00:00Z",
		time: 1,
		content: { kind: "text", text: "Here." },
		mentions: [],
		ui: [
			{
				kind: "ui",
				id: "ui1",
				a2ui: {
					type: "app_widget",
					origin: { app_id: "app1", board_id: "b1" },
					widget,
				},
			},
			{ kind: "ui", id: "ui2", handle },
		],
		edited: false,
		reactions: [],
	};
	const own = roomChatMessage(message, "room1", { ownBot: true });
	expect(own.widgets).toEqual([
		{ ...widget, origin: { appId: "app1", boardId: "b1" } },
	]);
	expect(own.widget_documents).toEqual([handle]);
	expect(own.widget_actions).toBeUndefined();

	const foreign = roomChatMessage(message, "room1");
	expect(foreign.widgets).toEqual([widget]);
	expect(foreign.widget_documents).toEqual([handle]);
	expect(foreign.widget_actions).toBe(false);
});
