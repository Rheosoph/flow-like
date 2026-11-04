// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import type {
	IConversationSegment,
	IInboxEntriesPage,
	IInboxEntriesQuery,
	IInboxEntry,
	IInboxImportEntry,
	IInboxState,
	ISendInboxEntryRequest,
} from "../backend-state/inbox-state";
import {
	importLegacyChat,
	interactiveEnvelope,
	jumpList,
	legacyImportEntry,
	loadAround,
	loadLatest,
	loadOlder,
	pushedTranscript,
	sendInteractive,
	startNewTopic,
	transcriptOf,
} from "./bot-history";
import { globalChatDb } from "./global-chat-db";

const BOT = "bot_ada";
const CONVERSATION = "conv_1";
const target = { botId: BOT, conversationId: CONVERSATION };

function entry(
	seq: number,
	sender: string,
	text: string,
	extra: Partial<IInboxEntry> = {},
): IInboxEntry {
	return {
		id: `e${seq}`,
		message_id: `m${seq}`,
		conversation_id: CONVERSATION,
		sender_id: sender,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text } },
		created_at: new Date(Date.UTC(2026, 10, 1, 12, seq)).toISOString(),
		cursor: `${seq * 1000}:e${seq}`,
		seq,
		...extra,
	};
}

/** A history of `count` alternating messages; reads by position like both backends do. */
function fakeInbox(count: number) {
	const entries = Array.from({ length: count }, (_, index) =>
		entry(index + 1, index % 2 === 0 ? "user_1" : BOT, `message ${index + 1}`),
	);
	const sent: ISendInboxEntryRequest[] = [];
	const imported = new Map<string, IInboxImportEntry>();
	const queries: IInboxEntriesQuery[] = [];
	const inbox = {
		async listEntries(
			_conversationId: string,
			query: IInboxEntriesQuery = {},
		): Promise<IInboxEntriesPage> {
			queries.push(query);
			const limit = query.limit ?? 50;
			if (query.around_seq !== undefined) {
				const from = Math.max(1, query.around_seq - (query.before_count ?? 10));
				const to = Math.min(
					count,
					query.around_seq + (query.after_count ?? 10),
				);
				return {
					entries: entries.slice(from - 1, to),
					next_before: null,
					next_to_seq: from > 1 ? from - 1 : null,
					next_from_seq: to < count ? to + 1 : null,
				};
			}
			const to = Math.min(query.to_seq ?? count, count);
			const from = Math.max(1, to - limit + 1);
			return {
				entries: entries.slice(from - 1, to),
				next_before: null,
				next_to_seq: from > 1 ? from - 1 : null,
				next_from_seq: null,
			};
		},
		async sendEntry(_id: string, request: ISendInboxEntryRequest) {
			sent.push(request);
			return entry(count + sent.length, "user_1", "", {
				message_id: request.message_id,
				kind: request.kind,
			});
		},
		async listSegments() {
			return { segments: [] as IConversationSegment[], next_after: null };
		},
		async importHistory(
			_botId: string,
			_conversationId: string,
			batch: IInboxImportEntry[],
		) {
			let added = 0;
			for (const item of batch) {
				if (!imported.has(item.message_id)) {
					imported.set(item.message_id, item);
					added += 1;
				}
			}
			return { imported: added, skipped: batch.length - added };
		},
	};
	return { inbox: inbox as unknown as IInboxState, sent, imported, queries };
}

describe("the chat from the bot's inbox history", () => {
	test("reads the newest page by position and maps senders to roles", async () => {
		const { inbox, queries } = fakeInbox(120);
		const page = await loadLatest(inbox, target, "bot:bot_ada", 50);
		expect(queries[0]).toMatchObject({ limit: 50 });
		expect(queries[0].to_seq).toBeGreaterThan(120);
		expect(page.messages).toHaveLength(50);
		expect(page.messages[0].id).toBe("m71");
		expect(page.messages.at(-1)?.id).toBe("m120");
		expect(page.messages[0].inner.role).toBe(IRole.User);
		expect(page.messages[1].inner.role).toBe(IRole.Assistant);
		expect(page.messages[0].sessionId).toBe("bot:bot_ada");
		expect(page.seqs.m71).toBe(71);
		expect(page.olderTo).toBe(70);
	});

	test("pages back by position until the start of the history", async () => {
		const { inbox } = fakeInbox(120);
		const latest = await loadLatest(inbox, target, "bot:bot_ada", 50);
		const older = await loadOlder(
			inbox,
			target,
			"bot:bot_ada",
			latest.olderTo ?? 0,
			50,
		);
		expect(older.messages[0].id).toBe("m21");
		const oldest = await loadOlder(
			inbox,
			target,
			"bot:bot_ada",
			older.olderTo ?? 0,
			50,
		);
		expect(oldest.messages.map((message) => message.id)).toHaveLength(20);
		expect(oldest.olderTo).toBeNull();
	});

	test("a jump loads a window around the position", async () => {
		const { inbox, queries } = fakeInbox(200);
		const page = await loadAround(inbox, target, "bot:bot_ada", 100);
		expect(queries[0]).toMatchObject({ around_seq: 100 });
		expect(page.messages.some((message) => message.id === "m100")).toBe(true);
		expect(page.messages.at(-1)?.id).toBe("m110");
		expect(page.newerFrom).toBe(111);
	});

	test("edits and deletes apply, topics and reactions are not messages", () => {
		const entries = [
			entry(1, "user_1", "hello"),
			entry(2, BOT, "draft"),
			entry(3, BOT, "final", {
				kind: "edit",
				message_id: "m3",
				envelope: { v: 1, enc: "none", body: { target: "m2", text: "final" } },
			}),
			entry(4, "user_1", "", {
				kind: "topic",
				envelope: { v: 1, enc: "none", body: { title: "Trip" } },
			}),
			entry(5, "user_1", "gone"),
			entry(6, "user_1", "", {
				kind: "delete",
				message_id: "m6",
				envelope: { v: 1, enc: "none", body: { target: "m5" } },
			}),
		];
		const { messages } = transcriptOf(entries, BOT, "bot:bot_ada");
		expect(messages.map((message) => message.inner.content)).toEqual([
			"hello",
			"final",
		]);
	});

	test("a trigger and a bot ask show as notices; only the trigger reaches the model", () => {
		const entries = [
			entry(1, "user_1", "", {
				kind: "trigger",
				envelope: {
					v: 1,
					enc: "none",
					body: {
						source: "schedule",
						ref_id: "task_1",
						title: "Morning brief",
						text: "Summarize the news",
					},
				},
			}),
			entry(2, BOT, "", {
				kind: "activity",
				envelope: {
					v: 1,
					enc: "none",
					body: {
						type: "bot_ask",
						ask_id: "ask-1",
						asker_id: BOT,
						callee_id: "bot_pebble",
						exchange_id: "conv_x",
						question: "Any news?",
					},
				},
			}),
			entry(3, BOT, "Here is the news"),
		];
		const { messages, seqs } = transcriptOf(entries, BOT, "bot:bot_ada");
		expect(messages.map((message) => message.notice?.note.type)).toEqual([
			"trigger",
			"bot_ask",
			undefined,
		]);
		expect(messages[0].inner).toEqual({
			role: IRole.System,
			content: "Summarize the news",
		});
		expect(messages[1].inner.content).toBe("");
		expect(messages[1].notice?.botId).toBe(BOT);
		expect(seqs).toEqual({ m1: 1, m2: 2, m3: 3 });
	});

	test("reads further back while reactions leave the newest page short", async () => {
		const { inbox, queries } = fakeInbox(120);
		const listEntries = inbox.listEntries.bind(inbox);
		const reacting = {
			...inbox,
			async listEntries(id: string, query?: IInboxEntriesQuery) {
				const page = await listEntries(id, query);
				return {
					...page,
					entries: page.entries.map((item) =>
						(item.seq ?? 0) > 80
							? {
									...item,
									kind: "reaction" as const,
									envelope: {
										v: 1 as const,
										enc: "none" as const,
										body: { target: "m1", emoji: "👍", op: "add" },
									},
								}
							: item,
					),
				};
			},
		} as IInboxState;
		const page = await loadLatest(reacting, target, "bot:bot_ada", 50);
		expect(queries.map((query) => query.to_seq)).toEqual([
			Number.MAX_SAFE_INTEGER,
			70,
		]);
		expect(page.messages).toHaveLength(60);
		expect(page.messages.at(-1)?.id).toBe("m80");
		expect(page.olderTo).toBe(20);
		expect(page.newestCursor).toBe("120000:e120");
	});

	test("a page remembers the cursor of its newest entry", async () => {
		const { inbox } = fakeInbox(3);
		const page = await loadLatest(inbox, target, "bot:bot_ada");
		expect(page.newestCursor).toBe("3000:e3");
	});
});

describe("entries the realtime feed pushes", () => {
	const sessionId = "bot:bot_ada";
	const shown = (id: string, role: IRole, timestamp: number) => ({
		id,
		appId: "global",
		sessionId,
		inner: { role, content: id },
		files: [],
		timestamp,
	});

	test("the bot's stored reply to a message the chat streamed is left out", () => {
		const onScreen = [
			shown("m1", IRole.User, 1),
			shown("local-answer", IRole.Assistant, 2),
		];
		const pushed = [
			entry(1, "user_1", "hi"),
			entry(2, BOT, "hello", {
				envelope: {
					v: 1,
					enc: "none",
					body: { text: "hello", reply_to: "m1" },
				},
			}),
			entry(3, BOT, "", {
				kind: "trigger",
				envelope: {
					v: 1,
					enc: "none",
					body: { source: "webhook", ref_id: "h1", title: "Order", text: "Go" },
				},
			}),
			entry(4, BOT, "On it", {
				envelope: {
					v: 1,
					enc: "none",
					body: { text: "On it", reply_to: "m3" },
				},
			}),
		];
		const { messages } = pushedTranscript(pushed, BOT, sessionId, onScreen);
		expect(messages.map((message) => message.id)).toEqual(["m1", "m3", "m4"]);
	});

	test("a reply to a message still waiting for its answer comes through", () => {
		const onScreen = [shown("m1", IRole.User, 1)];
		const pushed = [
			entry(2, BOT, "hello", {
				envelope: {
					v: 1,
					enc: "none",
					body: { text: "hello", reply_to: "m1" },
				},
			}),
		];
		const { messages } = pushedTranscript(pushed, BOT, sessionId, onScreen);
		expect(messages.map((message) => message.id)).toEqual(["m2"]);
	});
});

describe("sending from the interactive chat", () => {
	test("writes the inbox entry with the interactive mark and the chat's message id", async () => {
		const { inbox, sent } = fakeInbox(2);
		await sendInteractive(inbox, target, {
			id: "msg_cuid",
			text: "Plan my week",
			clientRunId: "run_9",
		});
		expect(sent).toEqual([
			{
				message_id: "msg_cuid",
				kind: "message",
				envelope: {
					v: 1,
					enc: "none",
					body: {
						text: "Plan my week",
						interactive: { client_run_id: "run_9" },
					},
				},
			},
		]);
		expect(interactiveEnvelope("x", "r").body).toMatchObject({
			interactive: { client_run_id: "r" },
		});
	});

	test("New topic writes a topic entry, titled when asked", async () => {
		const { inbox, sent } = fakeInbox(2);
		await startNewTopic(inbox, target);
		await startNewTopic(inbox, target, "  Holiday  ");
		expect(sent.map((request) => request.kind)).toEqual(["topic", "topic"]);
		expect(sent[0].envelope?.body).toEqual({});
		expect(sent[1].envelope?.body).toEqual({ title: "Holiday" });
		expect(sent[0].message_id).not.toBe(sent[1].message_id);
	});
});

describe("the jump list", () => {
	const segment = (
		id: string,
		level: number,
		from: number,
		to: number,
		title: string | null,
	): IConversationSegment => ({
		id,
		author_id: BOT,
		level,
		seq_from: from,
		seq_to: to,
		t_from: new Date(Date.UTC(2026, 9, from)).toISOString(),
		t_to: new Date(Date.UTC(2026, 9, to)).toISOString(),
		title,
		summary: "s",
		parent_id: null,
		version: 1,
		created_at: new Date().toISOString(),
	});

	test("lists the finest summaries newest first with titles and dates", () => {
		const rows = jumpList(
			[
				segment("s0-1-10", 0, 1, 10, "Setup"),
				segment("s0-11-20", 0, 11, 20, null),
				segment("s1-1-20", 1, 1, 20, "Both"),
			],
			(from) => `Chat of ${from.getUTCDate()}`,
		);
		expect(rows.map((row) => [row.id, row.title, row.seqFrom])).toEqual([
			["s0-11-20", "Chat of 11", 11],
			["s0-1-10", "Setup", 1],
		]);
		expect(rows[1].from).toBe(Date.UTC(2026, 9, 1));
	});
});

describe("the import of the IndexedDB chat", () => {
	beforeEach(() => globalChatDb.markers.clear());

	const legacy = [
		{ id: "a1", role: "user" as const, text: "Hi", timestamp: 1 },
		{ id: "a2", role: "assistant" as const, text: "Hello!", timestamp: 2 },
		{ id: "a3", role: "user" as const, text: "   ", timestamp: 3 },
		{ id: "bad id", role: "user" as const, text: "x", timestamp: 4 },
	];

	test("keeps the message ids, drops empty and unaddressable ones, oldest first", () => {
		expect(legacyImportEntry(legacy[0])).toMatchObject({
			message_id: "a1",
			sender: "owner",
		});
		expect(legacyImportEntry(legacy[1])?.sender).toBe("bot");
		expect(legacyImportEntry(legacy[2])).toBeNull();
		expect(legacyImportEntry(legacy[3])).toBeNull();
	});

	test("imports once: a repeat run adds nothing, even without the done flag", async () => {
		const { inbox, imported } = fakeInbox(0);
		const spy = vi.spyOn(
			inbox as never as { importHistory: () => unknown },
			"importHistory",
		);
		expect(await importLegacyChat(inbox, target, [...legacy].reverse())).toBe(
			2,
		);
		expect([...imported.keys()]).toEqual(["a1", "a2"]);

		expect(await importLegacyChat(inbox, target, legacy)).toBe(0);
		expect(spy).toHaveBeenCalledTimes(1);

		await globalChatDb.markers.clear();
		expect(await importLegacyChat(inbox, target, legacy)).toBe(0);
		expect(imported.size).toBe(2);
	});

	test("messages this device saved after the import follow on the next run", async () => {
		const { inbox, imported } = fakeInbox(0);
		await importLegacyChat(inbox, target, legacy);
		const later = Date.now() + 1_000;
		const stranded = [
			...legacy,
			{
				id: "s1",
				role: "user" as const,
				text: "Still there?",
				timestamp: later,
			},
			{
				id: "s2",
				role: "assistant" as const,
				text: "Yes",
				timestamp: later + 1,
			},
		];
		expect(await importLegacyChat(inbox, target, stranded)).toBe(2);
		expect([...imported.keys()]).toEqual(["a1", "a2", "s1", "s2"]);
	});

	test("an empty old chat still marks the bot done", async () => {
		const { inbox } = fakeInbox(0);
		expect(await importLegacyChat(inbox, target, [])).toBe(0);
		expect(
			await globalChatDb.markers.get(`bot-chat-imported:${BOT}`),
		).toBeDefined();
	});
});
