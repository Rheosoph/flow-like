import { describe, expect, test } from "bun:test";
import { conversationRealtimeRoom } from "./conversation-room";

describe("conversationRealtimeRoom", () => {
	test("names the per-conversation room", () => {
		expect(conversationRealtimeRoom("c_123")).toBe("inbox:c_123:room-v1");
	});
});
