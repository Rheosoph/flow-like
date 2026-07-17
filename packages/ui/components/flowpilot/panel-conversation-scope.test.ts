import { describe, expect, it } from "vitest";
import { flowPilotPanelConversationId } from "./panel-conversation-scope";

function memoryStorage(): Pick<Storage, "getItem" | "setItem"> {
	const store = new Map<string, string>();
	return {
		getItem: (key) => store.get(key) ?? null,
		setItem: (key, value) => {
			store.set(key, value);
		},
	};
}

describe("flowPilotPanelConversationId", () => {
	it("is stable across calls for the same board", () => {
		const storage = memoryStorage();
		const first = flowPilotPanelConversationId("board-1", storage);
		const second = flowPilotPanelConversationId("board-1", storage);
		expect(first).toBe(second);
		expect(first.length).toBeGreaterThan(0);
	});

	it("is distinct per board", () => {
		const storage = memoryStorage();
		const boardA = flowPilotPanelConversationId("board-a", storage);
		const boardB = flowPilotPanelConversationId("board-b", storage);
		expect(boardA).not.toBe(boardB);
	});

	it("survives a simulated reload through the persisted storage entry", () => {
		const storage = memoryStorage();
		const before = flowPilotPanelConversationId("board-reload", storage);
		const after = flowPilotPanelConversationId("board-reload", storage);
		expect(after).toBe(before);
	});

	it("stays stable within a session when storage is unavailable", () => {
		const first = flowPilotPanelConversationId("board-no-storage", undefined);
		const second = flowPilotPanelConversationId("board-no-storage", undefined);
		expect(first).toBe(second);
	});
});
