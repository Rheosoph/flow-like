import "fake-indexeddb/auto";
import Dexie from "dexie";
import { afterEach, beforeEach, expect, test } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import {
	forgetMovedConversation,
	rememberMovedConversation,
} from "../../state/global-chat/global-chat-bots";
import type { IMessage } from "../../state/global-chat/global-chat-db";

Dexie.dependencies.indexedDB = indexedDB;
Dexie.dependencies.IDBKeyRange = IDBKeyRange;
const { GLOBAL_CHAT_APP_ID, globalChatDb } = await import(
	"../../state/global-chat/global-chat-db"
);
const { botMessageActivity, readLatestBotChatActivity } = await import(
	"./bots-nav-history"
);

function message(
	id: string,
	sessionId: string,
	timestamp: number,
	role = IRole.User,
): IMessage {
	return {
		id,
		appId: GLOBAL_CHAT_APP_ID,
		sessionId,
		timestamp,
		inner: { role, content: "Hello" },
		files: [],
	};
}

beforeEach(async () => {
	await globalChatDb.messages.clear();
});

afterEach(async () => {
	await globalChatDb.messages.clear();
	forgetMovedConversation("bot:old-assistant");
});

test("has no recent bot before the first message", async () => {
	expect(await readLatestBotChatActivity()).toBeNull();
});

test("follows the latest user message across Flux and custom bot chats", async () => {
	await globalChatDb.messages.bulkPut([
		message("first", "flux-history", 100),
		message("second", "bot:scout", 200),
	]);
	expect(await readLatestBotChatActivity()).toEqual({
		botId: "scout",
		at: 200,
	});
	await globalChatDb.messages.put(message("third", "flux-history", 300));
	expect(await readLatestBotChatActivity()).toEqual({ botId: null, at: 300 });
});

test("an older background run becomes latest when its reply finishes", async () => {
	const reply = message("reply", "bot:scout", 100, IRole.Assistant);
	await globalChatDb.messages.bulkPut([
		reply,
		message("later-send", "bot:poet", 200),
	]);
	expect(await readLatestBotChatActivity()).toEqual({ botId: "poet", at: 200 });
	await globalChatDb.messages.put({
		...reply,
		run_context: { schema: "flowpilot.run-context/v1", ended_at_ms: 300 },
	});
	expect(await readLatestBotChatActivity()).toEqual({
		botId: "scout",
		at: 300,
	});
});

test("skips malformed and unrelated messages in both activity indexes", async () => {
	const valid = message("valid", "bot:scout", 100);
	const irrelevant: IMessage[] = [
		{ ...message("app", "app-session", 200), appId: "app" },
		message("system", "bot:system", 300, IRole.System),
		message("tool", "bot:tool", 400, IRole.ITool),
		message("function", "bot:function", 500, IRole.Function),
		{
			...message("notice", "bot:notice", 600),
			notice: {} as IMessage["notice"],
		},
		message("empty-session", "", 700),
		message("blank-session", "   ", 800),
		message("infinite", "bot:infinite", Number.POSITIVE_INFINITY),
		message("nan", "bot:nan", Number.NaN),
		message("zero", "bot:zero", 0),
		message("negative", "bot:negative", -1),
		{
			...message("missing-role", "bot:missing-role", 900),
			inner: null,
		} as unknown as IMessage,
	];
	await globalChatDb.messages.bulkPut([
		valid,
		...irrelevant.map((item) => ({
			...item,
			run_context: { schema: "flowpilot.run-context/v1", ended_at_ms: 10_000 },
		})),
	]);
	expect(await readLatestBotChatActivity()).toEqual({
		botId: "scout",
		at: 100,
	});
});

test("ignores invalid completion times and completion metadata on user messages", async () => {
	const reply = message("reply", "bot:scout", 100, IRole.Assistant);
	for (const endedAt of [Number.NaN, Number.POSITIVE_INFINITY, -1, 0]) {
		expect(
			botMessageActivity({
				...reply,
				run_context: {
					schema: "flowpilot.run-context/v1",
					ended_at_ms: endedAt,
				},
			}),
		).toEqual({ botId: "scout", at: 100 });
	}
	await globalChatDb.messages.bulkPut([
		{
			...reply,
			run_context: { schema: "flowpilot.run-context/v1", ended_at_ms: 50 },
		},
		{
			...message("user", "bot:poet", 200),
			run_context: { schema: "flowpilot.run-context/v1", ended_at_ms: 10_000 },
		},
	]);
	expect(await readLatestBotChatActivity()).toEqual({ botId: "poet", at: 200 });
});

test("resolves activity to the bot that now owns a moved conversation", async () => {
	rememberMovedConversation("bot:old-assistant", "bot:account-assistant");
	await globalChatDb.messages.put(message("old", "bot:old-assistant", 100));
	expect(await readLatestBotChatActivity()).toEqual({
		botId: "account-assistant",
		at: 100,
	});
});

test("upgrades existing history with the completion index while preserving its stores", async () => {
	await globalChatDb.delete();
	const previous = new Dexie("Global-Chat-History");
	previous.version(4).stores({
		sessions: "id, updatedAt, boardId, mode, pinnedAt",
		messages: "id, sessionId, timestamp",
		markers: "key",
	});
	await previous.table("messages").bulkPut([
		message("newer", "bot:poet", 200),
		{
			...message("reply", "bot:scout", 100, IRole.Assistant),
			run_context: { schema: "flowpilot.run-context/v1", ended_at_ms: 300 },
		},
	]);
	await previous
		.table("sessions")
		.put({ id: "kept", updatedAt: 50, pinnedAt: 10 });
	await previous.table("markers").put({ key: "kept", at: 50 });
	previous.close();
	await globalChatDb.open();
	expect(await readLatestBotChatActivity()).toEqual({
		botId: "scout",
		at: 300,
	});
	expect(await globalChatDb.sessions.get("kept")).toMatchObject({
		pinnedAt: 10,
	});
	expect(await globalChatDb.markers.get("kept")).toEqual({
		key: "kept",
		at: 50,
	});
	expect(
		globalChatDb.messages.schema.indexes.map((index) => index.name),
	).toEqual(["sessionId", "timestamp", "run_context.ended_at_ms"]);
	await globalChatDb.sessions.delete("kept");
	await globalChatDb.markers.delete("kept");
});
