import "fake-indexeddb/auto";
import { beforeEach, expect, test } from "vitest";
import { completeAppChatTurn } from "../../components/interfaces/chat-default/chat-activity";
import {
	type IMessage,
	chatDb,
	decodeMessageRow,
	promoteChatDrafts,
	putChatDraft,
	putChatMessage,
} from "../../components/interfaces/chat-default/chat-db";
import { persistWorkflowReactions } from "../../components/interfaces/chat-default/workflow-reactions";
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

async function savedRow(id = "answer", draft = false) {
	const row = await (draft ? chatDb.drafts : chatDb.messages).get(id);
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

const reactionEvent = (emoji: string, operation = "add", messageId?: string) =>
	({
		event_id: crypto.randomUUID(),
		event_type: "chat_reaction",
		payload: { emoji, operation, message_id: messageId },
	}) as ExecutionTask["events"][number];

test("workflow reactions persist on the triggering user message without a live view or empty reply", async () => {
	const user = {
		...message("question", "Please review this"),
		inner: { role: "user", content: "Please review this" } as IMessage["inner"],
		reactions: ["❤️"],
	};
	await putChatMessage(user);
	const run = task();
	if (!run.chat) throw new Error("Chat seed missing");
	run.chat.responseMessage.trigger_message_id = user.id;
	run.events = [reactionEvent("👍")];
	await persistExecutionChat(run, false);
	expect(decodeMessageRow(await savedRow(user.id)).workflowReactions).toEqual([
		"👍",
	]);
	run.events.push(reactionEvent("👍", "remove"), reactionEvent("🎉"));
	await persistExecutionChat(run, false);
	run.status = "completed";
	await persistExecutionChat(run, true);
	const saved = decodeMessageRow(await savedRow(user.id));
	expect(saved.workflowReactions).toEqual(["🎉"]);
	expect(saved.reactions).toEqual(["❤️"]);
	expect(await chatDb.messages.count()).toBe(1);
	expect(await chatDb.drafts.count()).toBe(0);
});

test("replayed reactions preserve ownership and draft recovery, and cannot cross conversation boundaries", async () => {
	const old = { ...message("old", "Earlier reply"), reactions: ["👍"] };
	await putChatMessage(old);
	await putChatDraft(old);
	await putChatMessage({
		...message("other", "Other conversation"),
		sessionId: "other-session",
	});
	await putChatMessage({
		...message("other-app", "Other app"),
		appId: "other-app",
	});
	const target = { appId: "app", sessionId: "session" };
	const events = [
		reactionEvent("👍", "add", old.id),
		reactionEvent("❤️", "add", "other"),
		reactionEvent("❤️", "add", "other-app"),
		reactionEvent("🎉", "toggle", old.id),
	];
	await persistWorkflowReactions(events, target);
	const row = await savedRow(old.id);
	await persistWorkflowReactions(events, target);
	expect((await chatDb.messages.get(old.id))?.rev).toBe(row.rev);
	expect(
		decodeMessageRow(await savedRow(old.id, true)).workflowReactions,
	).toEqual(["👍"]);
	expect(
		decodeMessageRow(await savedRow("other")).workflowReactions,
	).toBeUndefined();
	expect(
		decodeMessageRow(await savedRow("other-app")).workflowReactions,
	).toBeUndefined();
	await persistWorkflowReactions(
		[reactionEvent("👍", "remove", old.id)],
		target,
	);
	const saved = decodeMessageRow(await savedRow(old.id));
	expect(saved.workflowReactions).toEqual([]);
	expect(saved.reactions).toEqual(["👍"]);
	await promoteChatDrafts("session");
	expect(decodeMessageRow(await savedRow(old.id)).workflowReactions).toEqual(
		[],
	);
});

test("late workflow reactions cannot change a removed conversation", async () => {
	await putChatMessage(message("old", "Earlier reply"));
	await chatDb.sessions.delete("session");
	await persistWorkflowReactions([reactionEvent("👍", "add", "old")], {
		appId: "app",
		sessionId: "session",
	});
	expect(
		decodeMessageRow(await savedRow("old")).workflowReactions,
	).toBeUndefined();
});

test("an older run checkpoint cannot restore a reaction that a later run removed", async () => {
	await putChatMessage(message("question", "Earlier message"));
	const first = task("first-run");
	first.events = [reactionEvent("👍", "add", "question")];
	await persistExecutionChat(first, false);
	const second = task("second-run");
	second.events = [reactionEvent("👍", "remove", "question")];
	await persistExecutionChat(second, true);
	await persistExecutionChat(first, true);
	const saved = decodeMessageRow(await savedRow("question"));
	expect(saved.workflowReactions).toEqual([]);
	expect(saved.workflowReactionEventIds).toEqual([
		first.events[0].event_id,
		second.events[0].event_id,
	]);
});

test("finalization preserves reaction state on a draft-only target even after its body advances", async () => {
	const reply = message("answer", "Still streaming");
	await putChatDraft(reply);
	const events = [reactionEvent("👍", "add", reply.id)];
	await persistWorkflowReactions(events, {
		appId: "app",
		sessionId: "session",
	});
	const run = task();
	await persistExecutionChat(run, true);
	const saved = decodeMessageRow(await savedRow(reply.id));
	expect(saved.workflowReactions).toEqual(["👍"]);
	expect(saved.workflowReactionEventIds).toEqual([events[0].event_id]);
	expect(saved.inner.content).toBe("First update");
});
