import { describe, expect, test } from "bun:test";
import type { IHistoryMessage, IResponse } from "../../lib";
import type { IAIState } from "../../state/backend-state/ai-state";
import { completeEditorChat, streamEditorChat } from "./ai-transport";

describe("editor AI usage attribution", () => {
	test("forwards app scope through streaming and completion transports", async () => {
		const calls: Array<{ appId?: string; mode: string }> = [];
		const response = {} as IResponse;
		const aiState: IAIState = {
			streamChatComplete: async (_messages, appId) => {
				calls.push({ appId, mode: "stream" });
				return new ReadableStream();
			},
			chatComplete: async (_messages, appId) => {
				calls.push({ appId, mode: "complete" });
				return response;
			},
		};
		const messages = [] as IHistoryMessage[];

		await streamEditorChat(aiState, messages, "app-1");
		await completeEditorChat(aiState, messages, "app-1");

		expect(calls).toEqual([
			{ appId: "app-1", mode: "stream" },
			{ appId: "app-1", mode: "complete" },
		]);
	});
});
