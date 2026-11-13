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
