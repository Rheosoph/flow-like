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
	expect(roomChatMessage(message, "room").storageBotId).toBeUndefined();
});
