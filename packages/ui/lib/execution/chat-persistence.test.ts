import "fake-indexeddb/auto";
import { beforeEach, expect, test } from "vitest";
import { completeAppChatTurn } from "../../components/interfaces/chat-default/chat-activity";
import {
	type IMessage,
	chatDb,
	decodeMessageRow,
	promoteChatDrafts,
	putChatDraft,
} from "../../components/interfaces/chat-default/chat-db";
import { persistExecutionChat } from "./chat-persistence";
import type { ExecutionTask } from "./types";

const message = (id = "answer", content = ""): IMessage => ({
	id,
	appId: "app",
	sessionId: "session",
	files: [],
	timestamp: 1,
	inner: { role: "assistant", content } as IMessage["inner"],
});

async function savedRow() {
	const row = await chatDb.messages.get("answer");
	if (!row) throw new Error("The assistant reply was not persisted");
	return row;
}

function task(id = crypto.randomUUID()): ExecutionTask {
	return {
		taskId: id,
		scope: "account",
		streamId: "session",
		appId: "app",
		eventId: "chat",
		title: "Chat",
		startedAt: new Date().toISOString(),
		status: "running",
		pendingInteractions: [],
		chat: { responseMessage: message() },
		events: [
			{
				event_id: "push",
				event_type: "chat_stream",
				payload: {
					response: {
						choices: [
							{
								index: 0,
								message: { role: "assistant", content: "First update" },
							},
						],
					},
				},
			},
		] as ExecutionTask["events"],
	};
}

beforeEach(async () => {
	await Promise.all([
		chatDb.messages.clear(),
		chatDb.drafts.clear(),
		chatDb.sessions.clear(),
		chatDb.localStage.clear(),
		chatDb.globalState.clear(),
	]);
	await chatDb.sessions.put({
		id: "session",
		appId: "app",
		summarization: "Chat",
		createdAt: 1,
		updatedAt: 1,
	});
});

test("reload promotes a newer draft over a visible response that was still streaming", async () => {
	await completeAppChatTurn(
		[message("answer", "First update")],
		{ appId: "app", eventId: "chat", sessionId: "session" },
		{ streaming: true },
	);
	await putChatDraft(message("answer", "First update plus later tokens"));
	await promoteChatDrafts("session");
	expect(decodeMessageRow(await savedRow()).inner.content).toBe(
		"First update plus later tokens",
	);
	expect(await chatDb.drafts.count()).toBe(0);
});

test("a runtime checkpoints a reply and finalizes the same identity without a chat subscriber", async () => {
	const run = task();
	await persistExecutionChat(run, false);
	expect((await chatDb.messages.get("answer"))?.streaming).toBe(true);
	run.events.push({
		event_id: "last",
		event_type: "chat_stream_partial",
		payload: {
			chunk: {
				id: "token",
				choices: [{ index: 0, delta: { content: " plus later tokens" } }],
			},
		},
	} as ExecutionTask["events"][number]);
	run.status = "completed";
	await persistExecutionChat(run, true);
	const row = await savedRow();
	expect(decodeMessageRow(row).inner.content).toBe(
		"First update plus later tokens",
	);
	expect(row.streaming).toBeUndefined();
	expect(await chatDb.messages.count()).toBe(1);
	expect(await chatDb.drafts.count()).toBe(0);
});

test("finalizing one run preserves another run's checkpoint and user reactions", async () => {
	const run = task();
	await persistExecutionChat(run, false);
	const row = await savedRow();
	const saved = decodeMessageRow(row);
	saved.reactions = ["thumbsup"];
	await chatDb.messages.put({ ...row, payload: JSON.stringify(saved) });
	await putChatDraft(message("next-run", "New reply"));
	run.status = "completed";
	await persistExecutionChat(run, true);
	expect(await chatDb.drafts.get("next-run")).toBeDefined();
	expect(decodeMessageRow(await savedRow()).reactions).toEqual(["thumbsup"]);
});

test("a removed conversation is not recreated by a late runtime save", async () => {
	await chatDb.sessions.delete("session");
	await persistExecutionChat(task(), true);
	expect(await chatDb.messages.count()).toBe(0);
	expect(await chatDb.drafts.count()).toBe(0);
});

test("cancellation preserves partial text with a stopped state", async () => {
	const run = task();
	run.status = "cancelled";
	await persistExecutionChat(run, true);
	const saved = decodeMessageRow(await savedRow());
	expect(saved.inner.content).toBe("First update");
	expect(saved.error?.kind).toBe("cancelled");
});
