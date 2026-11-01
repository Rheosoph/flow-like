import "fake-indexeddb/auto";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import * as activity from "../chat-activity-store";
import { globalChatDb } from "./global-chat-db";
import { useGlobalChatStore } from "./global-chat-store";
import {
	deleteGlobalChatConversation,
	driveGlobalChatStream,
	makeGlobalChatMessage,
	persistGlobalChatMessage,
	persistGlobalChatSession,
} from "./global-chat-stream";

let record = vi.spyOn(activity, "recordChatActivity");
let remove = vi.spyOn(activity, "removeChatActivity");

beforeEach(async () => {
	record = vi
		.spyOn(activity, "recordChatActivity")
		.mockImplementation(() => undefined);
	remove = vi
		.spyOn(activity, "removeChatActivity")
		.mockImplementation(() => {});
	await globalChatDb.messages.clear();
	await globalChatDb.sessions.clear();
	useGlobalChatStore.setState({
		activeConversationId: "visible-conversation",
		messages: [],
		runs: {},
		queue: [],
		draft: null,
		nativeDrafts: [],
	});
});

afterEach(() => {
	record.mockRestore();
	remove.mockRestore();
});

function reply(sessionId = "background-conversation") {
	return makeGlobalChatMessage(IRole.Assistant, "", sessionId);
}

test("a detached completion announces the saved reply under its conversation title", async () => {
	const responseMessage = reply();
	await persistGlobalChatSession(responseMessage.sessionId, "Review my orders");
	let savedAtNotification: Promise<unknown> | undefined;
	record.mockImplementation(() => {
		savedAtNotification = globalChatDb.messages.get(responseMessage.id);
		return undefined;
	});
	await driveGlobalChatStream({
		responseMessage,
		label: "A later question",
		start: async (onChunk) => {
			onChunk("Your order ");
			expect(record).not.toHaveBeenCalled();
			onChunk("arrives Tuesday.");
			return undefined;
		},
	});
	expect(record).toHaveBeenCalledTimes(1);
	expect(record.mock.calls[0][0]).toEqual({
		target: { kind: "flowpilot", sessionId: responseMessage.sessionId },
		messageId: responseMessage.id,
		title: "Review my orders",
		preview: "Your order arrives Tuesday.",
	});
	expect(await savedAtNotification).toMatchObject({
		inner: { content: "Your order arrives Tuesday." },
	});
	expect(useGlobalChatStore.getState().messages).toEqual([]);
});

test("checkpoint and metadata writes do not announce a reply", async () => {
	const message = reply();
	message.inner.content = "Still working";
	await persistGlobalChatMessage(message);
	await persistGlobalChatMessage({ ...message, rating: 1 });
	expect(record).not.toHaveBeenCalled();
});

test("cancelled turns and missed resumes do not announce new replies", async () => {
	await driveGlobalChatStream({
		responseMessage: reply(),
		start: async (onChunk) => {
			onChunk("Partial answer");
			throw new DOMException("Stopped", "AbortError");
		},
	});
	await driveGlobalChatStream({
		responseMessage: reply(),
		isResume: true,
		start: async () => ({ attached: false }),
	});
	expect(record).not.toHaveBeenCalled();
});

test("a failed final save does not offer a reply that cannot be restored", async () => {
	const put = vi
		.spyOn(globalChatDb.messages, "put")
		.mockRejectedValueOnce(new Error("Storage unavailable"));
	try {
		await driveGlobalChatStream({
			responseMessage: reply(),
			start: async () => ({ message: "Completed response" }),
		});
		expect(record).not.toHaveBeenCalled();
	} finally {
		put.mockRestore();
	}
});

test("deleting a live conversation removes its activity and blocks its closing notification", async () => {
	const responseMessage = reply("deleted-conversation");
	let finish!: () => void;
	const pending = new Promise<void>((resolve) => {
		finish = resolve;
	});
	const running = driveGlobalChatStream({
		responseMessage,
		start: async () => {
			await pending;
			return { message: "Late answer" };
		},
	});
	await deleteGlobalChatConversation(responseMessage.sessionId);
	finish();
	await running;
	expect(remove).toHaveBeenCalledWith({
		kind: "flowpilot",
		sessionId: responseMessage.sessionId,
	});
	expect(record).not.toHaveBeenCalled();
	expect(await globalChatDb.messages.get(responseMessage.id)).toBeUndefined();
});
