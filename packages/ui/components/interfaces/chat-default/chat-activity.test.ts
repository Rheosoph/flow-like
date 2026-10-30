import "fake-indexeddb/auto";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { buildChatMessageError } from "../../../lib/flowpilot/chat-error";
import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import { IRole } from "../../../lib/schema/llm/history";
import * as activity from "../../../state/chat-activity-store";
import {
	completeAppChatTurn,
	createAppChatActivityWriter,
} from "./chat-activity";
import {
	type IMessage,
	chatDb,
	decodeMessageRow,
	latestChatDraft,
	putChatDraft,
} from "./chat-db";
import { createChatTurn } from "./chat-turn";

const target = { appId: "app", eventId: "event", sessionId: "session" };
let record = vi.spyOn(activity, "recordChatActivity");

beforeEach(async () => {
	record = vi
		.spyOn(activity, "recordChatActivity")
		.mockImplementation(() => undefined);
	await chatDb.messages.clear();
	await chatDb.drafts.clear();
	await chatDb.sessions.clear();
	await chatDb.sessions.put({
		id: target.sessionId,
		appId: target.appId,
		summarization: "Watch my deployment",
		createdAt: 1,
		updatedAt: 1,
	});
});

afterEach(() => record.mockRestore());

function reply(id: string, content: string): IMessage {
	return {
		id,
		appId: target.appId,
		sessionId: target.sessionId,
		inner: { role: IRole.Assistant, content },
		timestamp: 1,
		files: [],
	};
}

test("persists every pushed message and announces the latest visible answer", async () => {
	await completeAppChatTurn(
		[reply("first", "Deployment started."), reply("last", "Deployment ready.")],
		target,
	);
	expect(await chatDb.messages.count()).toBe(2);
	expect(record).toHaveBeenCalledTimes(1);
	expect(record.mock.calls[0][0]).toEqual({
		target: { kind: "app", ...target },
		messageId: "last",
		title: "Watch my deployment",
		preview: "Deployment ready.",
	});
});

test("does not announce checkpoints, missing sessions, or ephemeral device chats", async () => {
	await chatDb.sessions.delete(target.sessionId);
	await completeAppChatTurn([reply("deleted", "Finished")], target);
	await completeAppChatTurn([reply("runtime", "Finished")], {
		...target,
		appId: "device-runtime:test",
	});
	expect(record).not.toHaveBeenCalled();
});

test("a cancelled multi-message turn stays saved without announcing an earlier fragment", async () => {
	const last = reply("last", "Partial response");
	last.error = buildChatMessageError(
		"bits",
		new DOMException("Stopped", "AbortError"),
	);
	await completeAppChatTurn([reply("first", "Started"), last], target);
	expect(await chatDb.messages.count()).toBe(2);
	expect(record).not.toHaveBeenCalled();
});

test("background failures provide a saved error even with no answer text", async () => {
	const message = reply("failed", "");
	message.error = buildChatMessageError(
		"bits",
		new Error("Network unavailable"),
	);
	await completeAppChatTurn([message], target);
	const row = await chatDb.messages.get(message.id);
	expect(row && decodeMessageRow(row).error).toEqual(message.error);
	expect(record.mock.calls[0][0].preview).toBe(message.error.message);
});

test("does not notify when the final transcript cannot be saved", async () => {
	const save = vi
		.spyOn(chatDb.messages, "bulkPut")
		.mockRejectedValueOnce(new Error("Storage unavailable"));
	try {
		await expect(
			completeAppChatTurn([reply("lost", "Finished")], target),
		).rejects.toThrow("Storage unavailable");
		expect(record).not.toHaveBeenCalled();
	} finally {
		save.mockRestore();
	}
});

test("a pushed response can notify before its workflow ends without clearing streaming recovery", async () => {
	const draft = reply("running", "Still working");
	await putChatDraft(draft);
	await completeAppChatTurn([reply("pushed", "A new update")], target, {
		streaming: true,
	});
	expect(await chatDb.drafts.count()).toBe(1);
	expect(await chatDb.messages.count()).toBe(1);
	expect(record.mock.calls[0][0].preview).toBe("A new update");
});

test("the first pushed response checkpoints its root before a notification can reopen the live chat", async () => {
	const responseMessage = reply("original-root", "");
	const turn = createChatTurn({
		responseMessage,
		...target,
		tmpLocalState: null,
		tmpGlobalState: null,
	});
	const events = [
		{
			event_id: "push",
			event_type: "chat_stream",
			payload: {
				message_id: "first-push",
				response: {
					choices: [
						{
							index: 0,
							message: { role: "assistant", content: "A new update" },
						},
					],
				},
			},
		},
	] as IIntercomEvent[];
	const writer = createAppChatActivityWriter(turn, target, () => events);
	writer.onEvents(events);
	await writer.settled();
	const draft = await latestChatDraft(target.sessionId);
	expect(draft?.id).toBe("original-root");
	expect(record.mock.calls[0][0].messageId).toBe(draft?.id);
	if (!draft) throw new Error("Reply checkpoint missing");
	const replay = createChatTurn({
		responseMessage: {
			...draft,
			inner: { role: IRole.Assistant, content: "" },
		},
		...target,
		tmpLocalState: null,
		tmpGlobalState: null,
	});
	replay.fold(events);
	await completeAppChatTurn(replay.responseMessages, target);
	expect(await chatDb.messages.count()).toBe(1);
});

test("later live content advances the saved revision after a cloned push snapshot", async () => {
	const live = reply("same-response", "First update");
	const snapshot = structuredClone(live);
	await putChatDraft(snapshot);
	await completeAppChatTurn([snapshot], target, { streaming: true });
	const pushed = await chatDb.messages.get(live.id);
	live.inner.content = "First update. The deployment is ready.";
	await putChatDraft(live);
	await completeAppChatTurn([live], target);
	const finalized = await chatDb.messages.get(live.id);
	if (!pushed || pushed.rev === undefined || !finalized) {
		throw new Error("Expected both pushed and finalized message rows");
	}
	expect(finalized.rev).toBeGreaterThan(pushed.rev);
	expect(decodeMessageRow(finalized).inner.content).toBe(live.inner.content);
});
