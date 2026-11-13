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
	test("preserves bot usage metadata from reloaded replies and drops malformed records", () => {
		const usage = {
			step_name: "Assistant",
			stats: {
				usage: {
					prompt_tokens: 100,
					completion_tokens: 10,
					total_tokens: 110,
					prompt_tokens_details: { cached_tokens: 70, cache_write_tokens: 0 },
				},
				context: {
					estimated_tokens: 100,
					context_window: 1_000,
					reserved_output_tokens: 100,
				},
				compactions: [
					{
						kind: "turn",
						trigger: "hard",
						status: "failed",
						before_tokens: 100,
						duration_ms: 200,
					},
				],
			},
		};
		const reply = entry(
			"message",
			"bot_1",
			{ text: "Done", usage_stats: [usage, null, { stats: { usage: "bad" } }] },
			0,
		);
		expect(messages(foldInboxEntries([reply], "me"))[0].usageStats).toEqual([
			usage,
		]);
	});

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

	test("renders renames and picture changes as room lines that break a run", () => {
		const items = foldInboxEntries(
			[
				entry("message", "anna", { text: "before" }, 0, "a"),
				entry(
					"room",
					"anna",
					{ change: "title", title: " Launch crew ", by: "anna" },
					1,
					"r1",
				),
				entry(
					"room",
					"me",
					{ change: "title", removed: true, by: "me" },
					2,
					"r2",
				),
				entry("room", "me", { change: "image", by: "me" }, 3, "r3"),
				entry(
					"room",
					"me",
					{ change: "image", removed: true, by: "me" },
					4,
					"r4",
				),
				entry("room", "me", { change: "colour", by: "me" }, 5, "r5"),
				entry("message", "anna", { text: "after" }, 6, "b"),
			],
			"me",
		);
		const rooms = items.filter((item) => item.type === "room");
		expect(
			rooms.map((item) =>
				item.type === "room"
					? [item.id, item.change, item.title, item.removed, item.by]
					: [],
			),
		).toEqual([
			["r1", "title", "Launch crew", false, "anna"],
			["r2", "title", undefined, true, "me"],
			["r3", "image", undefined, false, "me"],
			["r4", "image", undefined, true, "me"],
		]);
		const blocks = groupConversationItems(items, { dayKey: () => "day" });
		expect(blocks.map((block) => block.type)).toEqual([
			"day",
			"group",
			"room",
			"room",
			"room",
			"room",
			"group",
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
			"note",
			"group",
		]);
	});

	test("a trigger keeps what the details show: text, payload, file and bot", () => {
		const handle = {
			app_id: "ws_1",
			path: "webhooks/hook_1/1.json",
			bytes: 2048,
			sha256: "ab",
			mime: "application/json",
		};
		const [card] = foldInboxEntries(
			[
				trigger(0, {
					source: "webhook",
					ref_id: "hook_1",
					title: "New order",
					text: "Check the order",
					payload: { content_type: "application/json", body: { id: 7 } },
					payload_handle: handle,
					bot_id: "bot_1",
				}),
			],
			"me",
		);
		expect(card).toMatchObject({
			type: "trigger",
			refId: "hook_1",
			text: "Check the order",
			payload: { content_type: "application/json", body: { id: 7 } },
			payloadFile: handle,
			botId: "bot_1",
		});
	});
});

describe("bot ask activities", () => {
	const ask = {
		type: "bot_ask",
		ask_id: "ask-1",
		asker_id: "bot_scout",
		callee_id: "bot_pebble",
		exchange_id: "conv_x",
		question: "Which issues are open?",
	};

	test("an activity folds into a note between the messages", () => {
		const items = foldInboxEntries(
			[
				entry("message", "bot_scout", { text: "Let me ask Pebble" }, 0),
				entry("activity", "bot_scout", ask, 1),
				entry("message", "bot_scout", { text: "Pebble says two" }, 2),
			],
			"me",
		);
		expect(items.map((item) => item.type)).toEqual([
			"message",
			"bot_ask",
			"message",
		]);
		expect(items[1]).toMatchObject({
			askerId: "bot_scout",
			calleeId: "bot_pebble",
			exchangeId: "conv_x",
			question: "Which issues are open?",
		});
		const blocks = groupConversationItems(items, { dayKey: () => "day" });
		expect(blocks.map((block) => block.type)).toEqual([
			"day",
			"group",
			"note",
			"group",
		]);
	});

	test("an activity of another type or without its bots is not shown", () => {
		const items = foldInboxEntries(
			[
				entry("activity", "bot_scout", { ...ask, type: "teleport" }, 0),
				entry("activity", "bot_scout", { ...ask, callee_id: "" }, 1),
			],
			"me",
		);
		expect(items).toEqual([]);
	});
});

describe("sent mail", () => {
	test("a message recording a sent mail names its To and Cc recipients", () => {
		const [mail, chat] = messages(
			foldInboxEntries([
				entry(
					"message",
					"bot_a",
					{
						text: "Hallo Felix",
						mail: {
							direction: "out",
							from: "joerg@flow-like.me",
							to: ["felix@example.com"],
							cc: ["team@example.com"],
							subject: "Testmail",
						},
					},
					0,
				),
				entry("message", "bot_a", { text: "Done" }, 1),
			]),
		);
		expect(mail.mailedTo).toEqual(["felix@example.com", "team@example.com"]);
		expect(texts([mail])).toEqual(["Hallo Felix"]);
		expect(chat.mailedTo).toBeUndefined();
	});

	test("an inbound or unreadable mail record is not shown as sent", () => {
		const items = messages(
			foldInboxEntries([
				entry(
					"message",
					"bot_a",
					{ text: "a", mail: { direction: "in", to: ["x@example.com"] } },
					0,
				),
				entry(
					"message",
					"bot_a",
					{ text: "b", mail: { direction: "out", to: "x@example.com" } },
					1,
				),
			]),
		);
		expect(items.map((message) => message.mailedTo)).toEqual([
			undefined,
			undefined,
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

	test("the full configuration patch survives inbox parsing", () => {
		const changes = {
			name: "Atlas",
			instructions: "Full instructions.\n".repeat(100),
		};
		const [card] = messages(
			foldInboxEntries([
				entry(
					"message",
					"bot_a",
					{
						approval: {
							...approval,
							tool: "bot_update_configuration",
							scope: "",
							configuration_changes: changes,
						},
					},
					0,
					"ap-1",
				),
			]),
		);
		expect(card.approval?.configurationChanges).toEqual(changes);
	});

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
			postedAt: BASE,
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

	describe("secret requests", () => {
		const secretCard = (secret: unknown) =>
			messages(
				foldInboxEntries([
					entry(
						"message",
						"bot_a",
						{
							text: "x",
							approval: {
								...approval,
								tool: "secret_request",
								scope: "",
								secret,
							},
						},
						0,
						"ap-1",
					),
				]),
			)[0].approval;

		test("a valid secret rides along on the card", () => {
			expect(
				secretCard({
					name: "API_KEY",
					description: "For the API",
					replace: true,
				})?.secret,
			).toEqual({ name: "API_KEY", description: "For the API", replace: true });
		});

		test("replace is omitted unless true", () => {
			expect(
				secretCard({ name: "API_KEY", description: "x", replace: false })
					?.secret,
			).toEqual({ name: "API_KEY", description: "x" });
		});

		test.each([
			["a lower-case name", { name: "api_key", description: "x" }],
			[
				"a name over 64 characters",
				{ name: `A${"B".repeat(64)}`, description: "x" },
			],
			["a missing description", { name: "API_KEY" }],
			["a non-string description", { name: "API_KEY", description: 3 }],
			["a non-object", "API_KEY"],
		])("%s is treated as no secret", (_, secret) => {
			const card = secretCard(secret);
			expect(card).toBeDefined();
			expect(card?.secret).toBeUndefined();
		});
	});

	describe("room invitations", () => {
		const inviteCard = (invite: unknown) =>
			messages(
				foldInboxEntries([
					entry(
						"message",
						"bot_a",
						{
							text: "x",
							approval: { ...approval, tool: "room_invite", scope: "", invite },
						},
						0,
						"ap-1",
					),
				]),
			)[0].approval;

		test("the person and the reason ride along on the card", () => {
			expect(
				inviteCard({ person: "ana.lima", reason: "Owns the sheet" })?.invite,
			).toEqual({ person: "ana.lima", reason: "Owns the sheet" });
		});

		test.each([
			["an empty person", { person: "", reason: "x" }],
			["a missing reason", { person: "ana" }],
			["a non-object", "ana"],
		])("%s is treated as no invitation", (_, invite) => {
			const card = inviteCard(invite);
			expect(card).toBeDefined();
			expect(card?.invite).toBeUndefined();
		});
	});

	describe("batch cards", () => {
		const read = (hash: string, page: number, extra = {}) => ({
			args_hash: hash,
			tool: "web_request",
			summary: `The bot wants to send a GET request to cisa.gov/alerts?p=${page}.`,
			scope: "web:cisa.gov:read",
			...extra,
		});
		const batch = {
			...approval,
			id: "ap-b",
			tool: "approval_batch",
			summary: "The bot wants to make 4 calls",
			scope: "",
			args_hash: "hb",
			items: [
				read("h1", 1),
				read("h2", 2, { calls: 2, after_web_content: true }),
				{ args_hash: "h3", tool: "shell_run", summary: "Run git", scope: "" },
				{ tool: "broken" },
				read("h1", 9),
			],
		};
		const cardOf = (...extra: IInboxEntry[]) =>
			messages(
				foldInboxEntries([
					entry("message", "bot_a", { text: "x", approval: batch }, 0, "ap-b"),
					...extra,
				]),
			)[0];

		test("carries its well-formed items once each, with counts and flags", () => {
			expect(cardOf().approval?.items).toEqual([
				{
					argsHash: "h1",
					tool: "web_request",
					summary: read("h1", 1).summary,
					scopes: ["web:cisa.gov:read"],
					calls: 1,
					afterWebContent: false,
				},
				{
					argsHash: "h2",
					tool: "web_request",
					summary: read("h2", 2).summary,
					scopes: ["web:cisa.gov:read"],
					calls: 2,
					afterWebContent: true,
				},
				{
					argsHash: "h3",
					tool: "shell_run",
					summary: "Run git",
					scopes: [],
					calls: 1,
					afterWebContent: false,
				},
			]);
		});

		test("an answer with items records each differing call", () => {
			const answered = cardOf(
				entry(
					"message",
					"me",
					{
						approval_response: {
							approval_id: "ap-b",
							decision: "once",
							items: [
								{ args_hash: "h3", decision: "deny" },
								{ args_hash: "h1", decision: "sometimes" },
							],
						},
					},
					1,
					"ar-b",
				),
			);
			expect(answered.approval?.answer).toBe("once");
			expect(answered.approval?.itemAnswers).toEqual({ h3: "deny" });
		});

		test("a card with one item reads as that call, like a card from before batches", () => {
			const [card] = messages(
				foldInboxEntries([
					entry(
						"message",
						"bot_a",
						{ text: "x", approval: { ...batch, items: [read("h2", 2)] } },
						0,
						"ap-b",
					),
				]),
			);
			expect(card.approval).toMatchObject({
				tool: "web_request",
				summary: read("h2", 2).summary,
				scopes: ["web:cisa.gov:read"],
			});
			expect(card.approval?.items).toBeUndefined();
		});

		test("a flagged single call keeps its flag", () => {
			const [card] = messages(
				foldInboxEntries([
					entry(
						"message",
						"bot_a",
						{
							text: "x",
							approval: { ...approval, after_web_content: true },
						},
						0,
						"ap-1",
					),
				]),
			);
			expect(card.approval?.afterWebContent).toBe(true);
		});

		test("the bot's cancel notice for the turn closes the card", () => {
			const cancel = (sender: string) =>
				entry(
					"message",
					sender,
					{ text: "You cancelled this task." },
					1,
					"cancel-turn_1",
				);
			expect(cardOf(cancel("bot_a")).approval?.cancelled).toBe(true);
			expect(cardOf(cancel("anna")).approval?.cancelled).toBeUndefined();
			expect(cardOf().approval?.cancelled).toBeUndefined();
		});
	});
});

describe("call entries", () => {
	test("a started call becomes a call block that its ended entry closes", () => {
		const items = foldInboxEntries([
			entry("call", "anna", { op: "started" }, 0, "call_1"),
			entry("message", "anna", { text: "hi" }, 1),
			entry(
				"call",
				"paul",
				{ op: "ended", call: "call_1", duration_ms: 720_000 },
				12,
			),
		]);
		const call = items.find((item) => item.type === "call");
		expect(call).toMatchObject({
			id: "call_1",
			senderId: "anna",
			durationMs: 720_000,
		});
		expect(items.filter((item) => item.type === "call")).toHaveLength(1);
		const blocks = groupConversationItems(items);
		expect(blocks.map((block) => block.type)).toEqual(["day", "call", "group"]);
	});

	test("a live call has no duration yet", () => {
		const [call] = foldInboxEntries([
			entry("call", "anna", { op: "started" }, 0, "call_1"),
		]);
		expect(call).toMatchObject({ type: "call", id: "call_1" });
		expect((call as { durationMs?: number }).durationMs).toBeUndefined();
	});

	test("ignores an ended entry for an unknown call and bodies it cannot read", () => {
		const items = foldInboxEntries([
			entry("call", "anna", { op: "ended", call: "nope", duration_ms: 5 }, 0),
			entry("call", "anna", { op: "ring" }, 1),
			entry("call", "anna", { op: "started" }, 2, "call_2"),
			entry(
				"call",
				"anna",
				{ op: "ended", call: "call_2", duration_ms: -4 },
				3,
			),
		]);
		expect(items).toHaveLength(1);
		expect(items[0]).toMatchObject({ id: "call_2", durationMs: 0 });
	});

	test("the first ended entry wins", () => {
		const [call] = foldInboxEntries([
			entry("call", "anna", { op: "started" }, 0, "call_1"),
			entry(
				"call",
				"anna",
				{ op: "ended", call: "call_1", duration_ms: 60_000 },
				1,
			),
			entry(
				"call",
				"paul",
				{ op: "ended", call: "call_1", duration_ms: 90_000 },
				2,
			),
		]);
		expect((call as { durationMs?: number }).durationMs).toBe(60_000);
	});
});

describe("transcript entries", () => {
	const spoken = (
		sender: string,
		text: string,
		minute: number,
		extra: Record<string, unknown> = {},
	) =>
		entry(
			"transcript",
			sender,
			{
				call: "call_1",
				text,
				offset_ms: 1000,
				duration_ms: 900,
				mentions: [],
				...extra,
			},
			minute,
		);

	test("folds a line with its call, offset and mentions", () => {
		const items = foldInboxEntries(
			[spoken("anna", "hey momo", 0, { mentions: ["bot_momo"] })],
			"me",
		);
		expect(items).toHaveLength(1);
		expect(items[0]).toMatchObject({
			type: "transcript",
			senderId: "anna",
			call: "call_1",
			text: "hey momo",
			offsetMs: 1000,
			mentions: ["bot_momo"],
			mine: false,
		});
	});

	test("drops empty, sealed and malformed lines", () => {
		expect(
			foldInboxEntries([
				spoken("anna", "   ", 0),
				entry("transcript", "anna", { call: "c" }, 1),
				{
					...spoken("anna", "x", 2),
					envelope: { v: 1, enc: "x25519", sealed: "..." },
				} as unknown as IInboxEntry,
			]),
		).toEqual([]);
	});

	test("one speaker's consecutive lines share a spoken block", () => {
		const blocks = groupConversationItems(
			foldInboxEntries([
				spoken("anna", "one", 0),
				spoken("anna", "two", 1),
				spoken("paul", "three", 2),
			]),
		);
		const spokenBlocks = blocks.filter((b) => b.type === "spoken");
		expect(spokenBlocks).toHaveLength(2);
		expect(
			spokenBlocks[0].type === "spoken" &&
				spokenBlocks[0].lines.map((l) => l.text),
		).toEqual(["one", "two"]);
	});

	test("a chat message between lines splits them and starts its own run", () => {
		const blocks = groupConversationItems(
			foldInboxEntries([
				spoken("anna", "one", 0),
				entry("message", "anna", { text: "typed" }, 1),
				entry("message", "anna", { text: "typed too" }, 2),
				spoken("anna", "two", 3),
			]),
		);
		expect(blocks.filter((b) => b.type !== "day").map((b) => b.type)).toEqual([
			"spoken",
			"group",
			"spoken",
		]);
		const group = blocks.find((b) => b.type === "group");
		expect(group?.type === "group" && group.messages).toHaveLength(2);
	});

	test("a spoken block ends the message run before it", () => {
		const blocks = groupConversationItems(
			foldInboxEntries([
				entry("message", "anna", { text: "a" }, 0),
				spoken("anna", "said", 1),
				entry("message", "anna", { text: "b" }, 2),
			]),
		);
		expect(blocks.filter((b) => b.type === "group")).toHaveLength(2);
	});
});

describe("attachment parts", () => {
	test("keeps stored and linked parts and drops unusable ones", () => {
		const handle = {
			app_id: "app1",
			path: "bot/x.pdf",
			bytes: 1,
			sha256: "s",
			mime: "application/pdf",
		};
		const [folded] = messages(
			foldInboxEntries(
				[
					entry(
						"message",
						"bot",
						{
							text: "here",
							parts: [
								{ kind: "attachment", id: "a1", name: "x.pdf", handle },
								{
									kind: "attachment",
									id: "a2",
									name: "y.pdf",
									url: "https://a.test/y.pdf",
									mime: "application/pdf",
								},
								{ kind: "attachment", id: "a3", name: "z.pdf", url: "ftp://z" },
								{ kind: "attachment", id: "a4", name: "w.pdf" },
							],
						},
						0,
					),
				],
				"me",
			),
		);
		expect(folded.attachments?.map((part) => part.id)).toEqual(["a1", "a2"]);
	});
});
