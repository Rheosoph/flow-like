import { describe, expect, test } from "bun:test";
import type {
	IInboxEntry,
	InboxEntryKind,
} from "../../state/backend-state/inbox-state";
import {
	type ConversationItem,
	type ConversationMessage,
	foldInboxEntries,
	groupConversationItems,
} from "./fold-entries";

const BASE = Date.UTC(2026, 10, 1, 9, 0, 0);
let sequence = 0;

function entry(
	kind: InboxEntryKind,
	sender: string,
	body: Record<string, unknown>,
	minute: number,
	messageId = `m${++sequence}`,
): IInboxEntry {
	const createdAt = new Date(BASE + minute * 60_000).toISOString();
	return {
		id: `e_${messageId}`,
		message_id: messageId,
		conversation_id: "c1",
		sender_id: sender,
		kind,
		envelope: { v: 1, enc: "none", body },
		created_at: createdAt,
		cursor: `${(BASE + minute * 60_000) * 1000}:e_${messageId}`,
	};
}

function messages(items: ConversationItem[]): ConversationMessage[] {
	return items.filter(
		(item): item is ConversationMessage => item.type === "message",
	);
}

function texts(items: ConversationItem[]): string[] {
	return messages(items).map((message) =>
		message.content.kind === "text"
			? message.content.text
			: message.content.kind,
	);
}

describe("foldInboxEntries", () => {
	test("orders newest-first pages chronologically and drops duplicate copies", () => {
		const first = entry("message", "anna", { text: "one" }, 0, "a");
		const second = entry("message", "me", { text: "two" }, 1, "b");
		const third = entry("message", "bot_1", { text: "three" }, 2, "c");
		const items = foldInboxEntries([third, second, first, third], "me");
		expect(texts(items)).toEqual(["one", "two", "three"]);
		expect(messages(items).map((message) => message.mine)).toEqual([
			false,
			true,
			false,
		]);
	});

	test("orders entries written at the same time by id code units, as the server does", () => {
		const at = (id: string, text: string): IInboxEntry => ({
			...entry("message", "anna", { text }, 0, id),
			id,
			cursor: `${BASE * 1000}:${id}`,
		});
		const items = foldInboxEntries(
			[at("a9x", "third"), at("a10x", "second"), at("B1", "first")],
			"me",
		);
		expect(texts(items)).toEqual(["first", "second", "third"]);
	});

	test("applies the latest edit from the sender only", () => {
		const original = entry("message", "anna", { text: "draft" }, 0, "a");
		const foreign = entry(
			"edit",
			"mallory",
			{ target: "a", text: "hijack" },
			1,
		);
		const first = entry("edit", "anna", { target: "a", text: "better" }, 2);
		const last = entry("edit", "anna", { target: "a", text: "best" }, 3);
		const items = foldInboxEntries([last, foreign, first, original], "me");
		expect(texts(items)).toEqual(["best"]);
		expect(messages(items)[0]?.edited).toBe(true);
	});

	test("hides messages their sender deleted", () => {
		const kept = entry("message", "anna", { text: "keep" }, 0, "a");
		const gone = entry("message", "anna", { text: "gone" }, 1, "b");
		const foreign = entry("delete", "jonas", { target: "a" }, 2);
		const own = entry("delete", "anna", { target: "b" }, 3);
		expect(texts(foldInboxEntries([kept, gone, foreign, own], "me"))).toEqual([
			"keep",
		]);
	});

	test("aggregates reactions per emoji and marks mine", () => {
		const message = entry("message", "bot_1", { text: "done" }, 0, "a");
		const reactions = [
			entry("reaction", "me", { target: "a", emoji: "👍", op: "add" }, 1),
			entry("reaction", "anna", { target: "a", emoji: "👍", op: "add" }, 2),
			entry("reaction", "anna", { target: "a", emoji: "🎉", op: "add" }, 3),
			entry("reaction", "anna", { target: "a", emoji: "🎉", op: "remove" }, 4),
			entry("reaction", "anna", { target: "a", emoji: "🚀", op: "add" }, 5),
			entry("reaction", "me", { target: "a", emoji: "👍", op: "add" }, 6),
		];
		const [folded] = messages(foldInboxEntries([message, ...reactions], "me"));
		expect(folded?.reactions).toEqual([
			{ emoji: "👍", count: 2, mine: true, reactorIds: ["me", "anna"] },
			{ emoji: "🚀", count: 1, mine: false, reactorIds: ["anna"] },
		]);
	});

	test("in a chat on this device, `local` is the signed-in reader", () => {
		const mine = entry("message", "local", { text: "from before" }, 0, "a");
		const theirs = entry("message", "bot_1", { text: "noted" }, 1, "b");
		const reaction = entry(
			"reaction",
			"local",
			{ target: "b", emoji: "👍", op: "add" },
			2,
		);
		const folded = messages(foldInboxEntries([mine, theirs, reaction], "me"));
		expect(folded.map((message) => message.mine)).toEqual([true, false]);
		expect(folded[1]?.reactions).toEqual([
			{ emoji: "👍", count: 1, mine: true, reactorIds: ["local"] },
		]);
	});

	test("removing my reaction clears the mark", () => {
		const message = entry("message", "anna", { text: "hi" }, 0, "a");
		const add = entry(
			"reaction",
			"me",
			{ target: "a", emoji: "👀", op: "add" },
			1,
		);
		const remove = entry(
			"reaction",
			"me",
			{ target: "a", emoji: "👀", op: "remove" },
			2,
		);
		const [folded] = messages(foldInboxEntries([remove, add, message], "me"));
		expect(folded?.reactions).toEqual([]);
	});

	test("renders member events as system lines", () => {
		const join = entry(
			"member",
			"felix",
			{ op: "join", principal_id: "bot_1", by: "felix" },
			0,
			"j",
		);
		const leave = entry(
			"member",
			"anna",
			{ op: "leave", principal_id: "anna" },
			1,
			"l",
		);
		const broken = entry("member", "anna", { op: "wave" }, 2, "x");
		expect(foldInboxEntries([join, leave, broken], "me")).toEqual([
			{
				type: "member",
				id: "j",
				createdAt: join.created_at,
				time: BASE,
				op: "join",
				principalId: "bot_1",
				by: "felix",
			},
			{
				type: "member",
				id: "l",
				createdAt: leave.created_at,
				time: BASE + 60_000,
				op: "leave",
				principalId: "anna",
				by: "anna",
			},
		]);
	});

	test("keeps sealed and unreadable messages as placeholders", () => {
		const sealed: IInboxEntry = {
			...entry("message", "anna", {}, 0, "s"),
			envelope: {
				v: 1,
				enc: "hpke-x25519-sha256-chacha20poly1305",
				key_id: "k1",
				enc_key: "a2V5",
				ct: "Y3Q=",
			},
		};
		const odd = entry("message", "anna", { image: "x" }, 1, "o");
		const sealedEdit: IInboxEntry = {
			...entry("edit", "anna", {}, 2),
			envelope: sealed.envelope,
		};
		expect(texts(foldInboxEntries([sealed, odd, sealedEdit], "me"))).toEqual([
			"sealed",
			"unsupported",
		]);
	});

	test("ignores effects whose target is not loaded", () => {
		const reaction = entry(
			"reaction",
			"anna",
			{ target: "older", emoji: "👍", op: "add" },
			1,
		);
		const message = entry("message", "anna", { text: "new" }, 2, "n");
		expect(texts(foldInboxEntries([message, reaction], "me"))).toEqual(["new"]);
	});
});

describe("groupConversationItems", () => {
	const utcDay = (time: number) => new Date(time).toISOString().slice(0, 10);

	test("groups consecutive messages from one sender within the window", () => {
		const items = foldInboxEntries(
			[
				entry("message", "bot_1", { text: "1" }, 0),
				entry("message", "bot_1", { text: "2" }, 1),
				entry("message", "bot_1", { text: "3" }, 4),
				entry("message", "bot_1", { text: "4" }, 20),
				entry("message", "anna", { text: "5" }, 21),
			],
			"me",
		);
		const blocks = groupConversationItems(items, { dayKey: utcDay });
		expect(
			blocks.map((block) =>
				block.type === "group"
					? `${block.senderId}:${block.messages.length}`
					: block.type,
			),
		).toEqual(["day", "bot_1:3", "bot_1:1", "anna:1"]);
	});

	test("member lines and new days break a group", () => {
		const items = foldInboxEntries(
			[
				entry("message", "anna", { text: "1" }, 0),
				entry("member", "anna", { op: "join", principal_id: "bot_1" }, 1),
				entry("message", "anna", { text: "2" }, 2),
				entry("message", "anna", { text: "3" }, 24 * 60),
			],
			"me",
		);
		const blocks = groupConversationItems(items, { dayKey: utcDay });
		expect(blocks.map((block) => block.type)).toEqual([
			"day",
			"group",
			"member",
			"group",
			"day",
			"group",
		]);
	});
});

describe("trigger entries", () => {
	const trigger = (minute: number, body: Record<string, unknown>) =>
		entry("trigger", "me", body, minute);

	test("a trigger folds into a card of its own, between the messages", () => {
		const items = foldInboxEntries(
			[
				entry("message", "me", { text: "Remind me daily" }, 0),
				trigger(5, {
					source: "schedule",
					ref_id: "task_1",
					title: "Morning brief",
					text: "Summarize the news",
				}),
				entry("message", "bot_1", { text: "Here is the news" }, 6),
			],
			"me",
		);
		expect(items.map((item) => item.type)).toEqual([
			"message",
			"trigger",
			"message",
		]);
		const card = items[1];
		expect(card).toMatchObject({
			type: "trigger",
			source: "schedule",
			title: "Morning brief",
		});
		expect(texts(items)).toEqual(["Remind me daily", "Here is the news"]);
	});

	test("a trigger with an unknown source or without a body is not shown", () => {
		const items = foldInboxEntries(
			[trigger(0, { source: "carrier-pigeon", title: "x" }), trigger(1, {})],
			"me",
		);
		expect(items).toEqual([]);
	});

	test("a trigger breaks a run of messages and gets its own block", () => {
		const items = foldInboxEntries(
			[
				entry("message", "bot_1", { text: "a" }, 0),
				trigger(1, { source: "webhook", title: "Order", text: "Go" }),
				entry("message", "bot_1", { text: "b" }, 2),
			],
			"me",
		);
		const blocks = groupConversationItems(items, { dayKey: () => "day" });
		expect(blocks.map((block) => block.type)).toEqual([
			"day",
			"group",
			"trigger",
			"group",
		]);
	});
});

describe("approval cards", () => {
	const approval = {
		id: "ap-1",
		turn_id: "turn_1",
		tool: "shell_run",
		summary: "Run git status",
		scope: "command:git,command:sh",
		args_hash: "h1",
	};

	test("a card carries its tool, summary and scopes, and stays a message with its text", () => {
		const [card] = messages(
			foldInboxEntries([
				entry(
					"message",
					"bot_a",
					{ text: "Run git status", approval },
					0,
					"ap-1",
				),
			]),
		);
		expect(card.approval).toEqual({
			id: "ap-1",
			turnId: "turn_1",
			tool: "shell_run",
			summary: "Run git status",
			scopes: ["command:git", "command:sh"],
		});
		expect(texts([card])).toEqual(["Run git status"]);
	});

	test("an answer folds into its card and is not a message of its own", () => {
		const items = foldInboxEntries([
			entry(
				"message",
				"bot_a",
				{ text: "Run git status", approval },
				0,
				"ap-1",
			),
			entry(
				"message",
				"me",
				{ approval_response: { approval_id: "ap-1", decision: "always" } },
				1,
				"ar-1",
			),
		]);
		expect(messages(items)).toHaveLength(1);
		expect(messages(items)[0].approval?.answer).toBe("always");
	});

	test("an unknown decision is ignored and the card stays open", () => {
		const items = foldInboxEntries([
			entry("message", "bot_a", { text: "x", approval }, 0, "ap-1"),
			entry(
				"message",
				"me",
				{ approval_response: { approval_id: "ap-1", decision: "maybe" } },
				1,
				"ar-1",
			),
		]);
		expect(messages(items)[0].approval?.answer).toBeUndefined();
	});

	test("a card that is never remembered has no scopes", () => {
		const [card] = messages(
			foldInboxEntries([
				entry(
					"message",
					"bot_a",
					{ text: "x", approval: { ...approval, scope: "" } },
					0,
					"ap-1",
				),
			]),
		);
		expect(card.approval?.scopes).toEqual([]);
	});
});
