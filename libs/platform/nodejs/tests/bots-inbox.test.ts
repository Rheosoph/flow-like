import { afterEach, describe, expect, spyOn, test } from "bun:test";
import {
	AuthError,
	type Conversation,
	FlowLikeClient,
	FlowLikeError,
	type InboxEntry,
	type InboxReactionBody,
	NotFoundError,
	inboxEntryText,
	newInboxMessageId,
	plainBody,
	plainEnvelope,
} from "../src/index.js";

const BASE = "https://flow.test/api/v1";
const MESSAGE_ID = /^msg_[0-9a-f]{32}$/;

const mocks: ReturnType<typeof spyOn>[] = [];
afterEach(() => {
	for (const mock of mocks.splice(0)) mock.mockRestore();
});

function transport(...replies: (() => Response)[]) {
	let call = 0;
	const mock = spyOn(globalThis, "fetch").mockImplementation(async () =>
		(
			replies[Math.min(call++, replies.length - 1)] ?? (() => Response.json({}))
		)(),
	);
	mocks.push(mock);
	return mock;
}
const json =
	(body: unknown, status = 200) =>
	() =>
		Response.json(body, { status });
const empty = () => new Response(null, { status: 204 });

const client = () =>
	new FlowLikeClient({ baseUrl: "https://flow.test", pat: "pat_owner" });

function sent(mock: ReturnType<typeof transport>, index = 0) {
	const [url, options] = mock.mock.calls[index];
	return {
		url: String(url),
		method: options?.method,
		headers: new Headers(options?.headers),
		body:
			typeof options?.body === "string" ? JSON.parse(options.body) : undefined,
	};
}

function entry(id: string, overrides: Partial<InboxEntry> = {}): InboxEntry {
	return {
		id,
		message_id: `m_${id}`,
		conversation_id: "conv",
		sender_id: "bot_1",
		kind: "message",
		envelope: plainEnvelope({ text: id }),
		created_at: "2026-11-02T10:00:00.000Z",
		cursor: `1700000000000000:${id}`,
		...overrides,
	};
}

const bot = {
	id: "bot_1",
	owner_id: "user_1",
	system: null,
	preset_id: null,
	name: "Researcher",
	description: null,
	instructions: null,
	appearance: { kind: "pip" },
	model: { primary: null, fallbacks: [] },
	classification: { pii: false },
	capabilities: ["internet"],
	profile_id: null,
	location: "server",
	device_id: null,
	import_key: null,
	created_at: "2026-11-02T10:00:00.000Z",
	updated_at: "2026-11-02T10:00:00.000Z",
};

const conversation: Conversation = {
	id: "conv",
	kind: "direct",
	title: null,
	encryption: "plain",
	created_by: "user_1",
	created_at: "2026-11-02T10:00:00.000Z",
	last_activity_at: "2026-11-02T10:00:00.000Z",
	members: [
		{
			principal_id: "user_1",
			kind: "user",
			added_by: null,
			joined_at: "2026-11-02T10:00:00.000Z",
		},
		{
			principal_id: "bot_1",
			kind: "bot",
			added_by: "user_1",
			joined_at: "2026-11-02T10:00:00.000Z",
		},
	],
	unread_count: 0,
	last_entry: null,
};

describe("bots", () => {
	test("listBots unwraps the list and authenticates with the PAT", async () => {
		const mock = transport(json({ bots: [bot] }));
		const bots = await client().listBots();
		expect(bots).toEqual([bot] as never);
		const request = sent(mock);
		expect(request).toMatchObject({ method: "GET", url: `${BASE}/bots` });
		expect(request.headers.get("authorization")).toBe("pat_owner");
		expect(request.headers.has("x-api-key")).toBe(false);
	});

	test("createBot sends snake_case fields and defaults the appearance", async () => {
		const mock = transport(json(bot));
		const sdk = client();
		await sdk.createBot({
			name: "Researcher",
			capabilities: ["internet", "selected_apps"],
			model: { primary: "bit_model", fallbacks: ["bit_fallback"] },
			classification: { classification: "internal", pii: true },
			profile_id: "profile",
		});
		expect(sent(mock)).toMatchObject({
			method: "POST",
			url: `${BASE}/bots`,
			body: {
				name: "Researcher",
				appearance: { kind: "pip" },
				capabilities: ["internet", "selected_apps"],
				model: { primary: "bit_model", fallbacks: ["bit_fallback"] },
				classification: { classification: "internal", pii: true },
				profile_id: "profile",
			},
		});
		await sdk.createBot({
			name: "Desk",
			appearance: { kind: "robo", frame: "round" },
			location: "desktop",
			capabilities: ["command_line"],
		});
		expect(sent(mock, 1).body).toEqual({
			name: "Desk",
			appearance: { kind: "robo", frame: "round" },
			location: "desktop",
			capabilities: ["command_line"],
		});
	});

	test("updateBot sends null to clear and omits untouched fields", async () => {
		const mock = transport(json(bot));
		await client().updateBot("bot /1", {
			description: null,
			instructions: "Answer briefly",
			capabilities: [],
			device_id: undefined,
		});
		expect(sent(mock)).toMatchObject({
			method: "PATCH",
			url: `${BASE}/bots/bot%20%2F1`,
		});
		expect(sent(mock).body).toEqual({
			description: null,
			instructions: "Answer briefly",
			capabilities: [],
		});
	});

	const routes: {
		name: string;
		invoke: (sdk: FlowLikeClient) => Promise<unknown>;
		method: string;
		path: string;
		body?: unknown;
		reply?: () => Response;
		result?: unknown;
	}[] = [
		{
			name: "get",
			invoke: (sdk) => sdk.getBot("bot_1"),
			method: "GET",
			path: "/bots/bot_1",
			reply: json(bot),
			result: bot,
		},
		{
			name: "delete",
			invoke: (sdk) => sdk.deleteBot("bot_1"),
			method: "DELETE",
			path: "/bots/bot_1",
			reply: empty,
		},
		{
			name: "list apps",
			invoke: (sdk) => sdk.listBotApps("bot_1"),
			method: "GET",
			path: "/bots/bot_1/apps",
			reply: json({
				apps: [{ app_id: "app", added_at: "t", owner_role: null }],
			}),
			result: [{ app_id: "app", added_at: "t", owner_role: null }],
		},
		{
			name: "attach app",
			invoke: (sdk) => sdk.attachBotApp("bot_1", "app/1"),
			method: "PUT",
			path: "/bots/bot_1/apps/app%2F1",
			reply: empty,
		},
		{
			name: "detach app",
			invoke: (sdk) => sdk.detachBotApp("bot_1", "app"),
			method: "DELETE",
			path: "/bots/bot_1/apps/app",
			reply: empty,
		},
		{
			name: "list secrets",
			invoke: (sdk) => sdk.listBotSecrets("bot_1"),
			method: "GET",
			path: "/bots/bot_1/secrets",
			reply: json({ secrets: [{ name: "API_TOKEN", encryption: "server" }] }),
			result: [{ name: "API_TOKEN", encryption: "server" }],
		},
		{
			name: "set secret value",
			invoke: (sdk) =>
				sdk.setBotSecret("bot_1", "API_TOKEN", {
					value: "s3cr3t",
					description: "CRM",
				}),
			method: "PUT",
			path: "/bots/bot_1/secrets/API_TOKEN",
			body: { value: "s3cr3t", description: "CRM" },
			reply: json({ name: "API_TOKEN", encryption: "server" }),
			result: { name: "API_TOKEN", encryption: "server" },
		},
		{
			name: "set sealed secret and clear its description",
			invoke: (sdk) =>
				sdk.setBotSecret("bot_1", "API_TOKEN", {
					sealed: { v: 1, enc: "hpke", key_id: "k", enc_key: "e", ct: "c" },
					description: null,
				}),
			method: "PUT",
			path: "/bots/bot_1/secrets/API_TOKEN",
			body: {
				sealed: { v: 1, enc: "hpke", key_id: "k", enc_key: "e", ct: "c" },
				description: null,
			},
			reply: json({ name: "API_TOKEN", encryption: "sealed" }),
			result: { name: "API_TOKEN", encryption: "sealed" },
		},
		{
			name: "delete secret",
			invoke: (sdk) => sdk.deleteBotSecret("bot_1", "API_TOKEN"),
			method: "DELETE",
			path: "/bots/bot_1/secrets/API_TOKEN",
			reply: empty,
		},
	];
	for (const route of routes)
		test(`route: ${route.name}`, async () => {
			const mock = transport(route.reply ?? json({}));
			const result = await route.invoke(client());
			const request = sent(mock);
			expect(request.method).toBe(route.method);
			expect(request.url).toBe(`${BASE}${route.path}`);
			expect(request.body).toEqual(route.body);
			expect(result).toEqual(route.result as never);
		});

	test("API errors keep the status, typed class and the server's code", async () => {
		transport(
			json(
				{ error: { code: "BOT_LIMIT_REACHED", message: "You have 50 bots" } },
				409,
			),
			json({ error: { code: "NOT_FOUND", message: "Bot 'x' not found" } }, 404),
			json(
				{ error: { code: "FORBIDDEN", message: "APIKey user is not allowed" } },
				403,
			),
		);
		const sdk = client();
		const limit = await sdk.createBot({ name: "One more" }).catch((e) => e);
		expect(limit).toBeInstanceOf(FlowLikeError);
		expect(limit.statusCode).toBe(409);
		expect(limit.message).toBe("You have 50 bots");
		expect(limit.body.error.code).toBe("BOT_LIMIT_REACHED");
		const missing = await sdk.getBot("x").catch((e) => e);
		expect(missing).toBeInstanceOf(NotFoundError);
		const refused = await sdk.listBots().catch((e) => e);
		expect(refused).toBeInstanceOf(AuthError);
		expect(refused.statusCode).toBe(403);
	});

	test("ids that would change the route are refused before sending", async () => {
		const mock = transport(json(bot));
		expect(() => client().getBot("..")).toThrow();
		expect(() => client().getBot("")).toThrow();
		expect(mock).not.toHaveBeenCalled();
	});
});

describe("inbox", () => {
	test("conversations are listed and read as yourself or as one of your bots", async () => {
		const mock = transport(json({ conversations: [conversation] }));
		const sdk = client();
		expect(await sdk.listConversations()).toEqual([conversation]);
		expect(sent(mock).url).toBe(`${BASE}/inbox/conversations`);
		await sdk.listConversations({ as: "bot_1" });
		expect(sent(mock, 1).url).toBe(`${BASE}/inbox/conversations?as=bot_1`);
		mock.mockImplementation(async () => Response.json(conversation));
		expect(await sdk.getConversation("conv/1", { as: "bot_1" })).toEqual(
			conversation,
		);
		expect(sent(mock, 2)).toMatchObject({
			method: "GET",
			url: `${BASE}/inbox/conversations/conv%2F1?as=bot_1`,
		});
	});

	test("conversation management routes and bodies", async () => {
		const mock = transport(json(conversation));
		const sdk = client();
		await sdk.createConversation({ kind: "direct", members: ["bot_1"] });
		await sdk.createConversation({
			kind: "group",
			title: "Launch",
			members: ["bot_1", "user_2"],
			encryption: "e2e",
		});
		await sdk.renameConversation("conv", "Renamed");
		await sdk.renameConversation("conv", null);
		await sdk.addConversationMembers("conv", ["bot_2"]);
		mock.mockImplementation(async () => empty());
		expect(await sdk.removeConversationMember("conv", "bot 2")).toBeUndefined();
		const calls = mock.mock.calls.map((_, index) => sent(mock, index));
		expect(
			calls.map(({ method, url, body }) => ({ method, url, body })),
		).toEqual([
			{
				method: "POST",
				url: `${BASE}/inbox/conversations`,
				body: { kind: "direct", members: ["bot_1"] },
			},
			{
				method: "POST",
				url: `${BASE}/inbox/conversations`,
				body: {
					kind: "group",
					title: "Launch",
					members: ["bot_1", "user_2"],
					encryption: "e2e",
				},
			},
			{
				method: "PATCH",
				url: `${BASE}/inbox/conversations/conv`,
				body: { title: "Renamed" },
			},
			{
				method: "PATCH",
				url: `${BASE}/inbox/conversations/conv`,
				body: { title: null },
			},
			{
				method: "POST",
				url: `${BASE}/inbox/conversations/conv/members`,
				body: { members: ["bot_2"] },
			},
			{
				method: "DELETE",
				url: `${BASE}/inbox/conversations/conv/members/bot%202`,
				body: undefined,
			},
		]);
	});

	test("listInboxEntries passes the window and exposes next_before", async () => {
		const page = { entries: [entry("b"), entry("a")], next_before: "1:a" };
		const mock = transport(json(page));
		const sdk = client();
		expect(
			await sdk.listInboxEntries("conv", {
				as: "bot_1",
				before: "9:z",
				after: "1:0",
				limit: 2,
			}),
		).toEqual(page);
		const url = new URL(sent(mock).url);
		expect(url.pathname).toBe("/api/v1/inbox/conversations/conv/entries");
		expect(Object.fromEntries(url.searchParams)).toEqual({
			as: "bot_1",
			before: "9:z",
			after: "1:0",
			limit: "2",
		});
		await sdk.listInboxEntries("conv");
		expect(sent(mock, 1).url).toBe(`${BASE}/inbox/conversations/conv/entries`);
	});

	test("iterateInboxEntries follows next_before until the history ends", async () => {
		const mock = transport(
			json({ entries: [entry("e"), entry("d")], next_before: "2:d" }),
			json({ entries: [entry("c"), entry("b")], next_before: "1:b" }),
			json({ entries: [entry("a")], next_before: null }),
		);
		const ids: string[] = [];
		for await (const item of client().iterateInboxEntries("conv", {
			as: "bot_1",
			after: "0:start",
			pageSize: 2,
		}))
			ids.push(item.id);
		expect(ids).toEqual(["e", "d", "c", "b", "a"]);
		expect(mock).toHaveBeenCalledTimes(3);
		const queries = mock.mock.calls.map((_, index) =>
			Object.fromEntries(new URL(sent(mock, index).url).searchParams),
		);
		expect(queries).toEqual([
			{ as: "bot_1", after: "0:start", limit: "2" },
			{ as: "bot_1", before: "2:d", after: "0:start", limit: "2" },
			{ as: "bot_1", before: "1:b", after: "0:start", limit: "2" },
		]);
	});

	test("leaving the history loop early stops paging", async () => {
		const mock = transport(
			json({ entries: [entry("b"), entry("a")], next_before: "1:a" }),
		);
		for await (const item of client().iterateInboxEntries("conv", {
			before: "9:z",
		})) {
			expect(item.id).toBe("b");
			break;
		}
		expect(mock).toHaveBeenCalledTimes(1);
		expect(new URL(sent(mock).url).searchParams.get("before")).toBe("9:z");
	});

	test("sendInboxMessage wraps text in a plain envelope with a generated message id", async () => {
		const mock = transport(json(entry("sent")));
		const sdk = client();
		expect(await sdk.sendInboxMessage("conv", "Hello")).toEqual(entry("sent"));
		const first = sent(mock);
		expect(first).toMatchObject({
			method: "POST",
			url: `${BASE}/inbox/conversations/conv/entries`,
			body: {
				kind: "message",
				envelope: { v: 1, enc: "none", body: { text: "Hello" } },
			},
		});
		expect(first.body.message_id).toMatch(MESSAGE_ID);
		expect(first.body.envelope.body).toEqual({ text: "Hello" });
		await sdk.sendInboxMessage("conv", "Again");
		expect(sent(mock, 1).body.message_id).not.toBe(first.body.message_id);
		await sdk.sendInboxMessage("conv", "Retry", {
			messageId: "msg_fixed",
			replyTo: "m_a",
		});
		expect(sent(mock, 2).body).toEqual({
			message_id: "msg_fixed",
			kind: "message",
			envelope: {
				v: 1,
				enc: "none",
				body: { text: "Retry", reply_to: "m_a" },
			},
		});
	});

	test("reactions, edits and deletions name their target message", async () => {
		const mock = transport(json(entry("x")));
		const sdk = client();
		await sdk.reactToInboxMessage("conv", "m_a", "👍");
		await sdk.reactToInboxMessage("conv", "m_a", "👍", {
			op: "remove",
			messageId: "msg_unreact",
		});
		await sdk.editInboxMessage("conv", "m_a", "Fixed typo");
		await sdk.deleteInboxMessage("conv", "m_a");
		const bodies = mock.mock.calls.map((_, index) => sent(mock, index).body);
		expect(bodies.map(({ kind, envelope }) => ({ kind, envelope }))).toEqual([
			{
				kind: "reaction",
				envelope: plainEnvelope({ target: "m_a", emoji: "👍", op: "add" }),
			},
			{
				kind: "reaction",
				envelope: plainEnvelope({ target: "m_a", emoji: "👍", op: "remove" }),
			},
			{
				kind: "edit",
				envelope: plainEnvelope({ target: "m_a", text: "Fixed typo" }),
			},
			{ kind: "delete", envelope: plainEnvelope({ target: "m_a" }) },
		]);
		expect(bodies[1].message_id).toBe("msg_unreact");
		for (const body of [bodies[0], bodies[2], bodies[3]])
			expect(body.message_id).toMatch(MESSAGE_ID);
	});

	test("sendInboxEntry passes sealed envelopes through and fills the message id", async () => {
		const mock = transport(json(entry("x")));
		const sealed = {
			v: 1 as const,
			enc: "hpke",
			key_id: "k",
			enc_key: "e",
			ct: "c",
		};
		await client().sendInboxEntry("conv", {
			kind: "message",
			envelopes: { user_1: sealed, bot_1: sealed },
		});
		const { body } = sent(mock);
		expect(body.envelopes).toEqual({ user_1: sealed, bot_1: sealed });
		expect(body.envelope).toBeUndefined();
		expect(body.message_id).toMatch(MESSAGE_ID);
	});

	test("send conflicts surface the server's code", async () => {
		transport(
			json(
				{
					error: {
						code: "INBOX_MESSAGE_ID_TAKEN",
						message: "This message id was already used for another event",
					},
				},
				409,
			),
		);
		const error = await client()
			.sendInboxMessage("conv", "Hi", { messageId: "msg_used" })
			.catch((e) => e);
		expect(error).toBeInstanceOf(FlowLikeError);
		expect(error.statusCode).toBe(409);
		expect(error.body.error.code).toBe("INBOX_MESSAGE_ID_TAKEN");
	});

	test("markConversationRead always sends a JSON body", async () => {
		const mock = transport(json({ last_read_at: "2026-11-02T10:00:00.000Z" }));
		const sdk = client();
		expect(await sdk.markConversationRead("conv")).toEqual({
			last_read_at: "2026-11-02T10:00:00.000Z",
		});
		await sdk.markConversationRead("conv", "5:e");
		expect(sent(mock)).toMatchObject({
			method: "POST",
			url: `${BASE}/inbox/conversations/conv/read`,
			body: {},
		});
		expect(sent(mock).headers.get("content-type")).toBe("application/json");
		expect(sent(mock, 1).body).toEqual({ up_to: "5:e" });
	});

	test("keys are fetched, registered and revoked", async () => {
		const key = {
			principal_id: "bot_1",
			key_id: "k:1",
			algorithm: "hpke-x25519-sha256-chacha20poly1305",
			public_key: "cHVi",
			created_at: "t",
		};
		const mock = transport(json({ keys: [key] }));
		const sdk = client();
		expect(await sdk.getInboxKeys(["user_1", "bot_1"])).toEqual([key]);
		expect(sent(mock).url).toBe(`${BASE}/inbox/keys?principals=user_1%2Cbot_1`);
		mock.mockImplementation(async () => Response.json(key));
		expect(
			await sdk.registerInboxKey({
				principal_id: "bot_1",
				key_id: "k:1",
				algorithm: key.algorithm,
				public_key: "cHVi",
			}),
		).toEqual(key);
		expect(sent(mock, 1)).toMatchObject({
			method: "PUT",
			url: `${BASE}/inbox/keys`,
			body: {
				principal_id: "bot_1",
				key_id: "k:1",
				algorithm: key.algorithm,
				public_key: "cHVi",
			},
		});
		mock.mockImplementation(async () => empty());
		expect(await sdk.revokeInboxKey("k:1", "bot_1")).toBeUndefined();
		await sdk.revokeInboxKey("k:2");
		expect(sent(mock, 2)).toMatchObject({
			method: "DELETE",
			url: `${BASE}/inbox/keys/k%3A1?principal_id=bot_1`,
		});
		expect(sent(mock, 3).url).toBe(`${BASE}/inbox/keys/k%3A2`);
	});
});

test("newInboxMessageId is unique and accepted by the server's message id rule", () => {
	const ids = new Set(Array.from({ length: 1000 }, newInboxMessageId));
	expect(ids.size).toBe(1000);
	for (const id of ids) expect(id).toMatch(MESSAGE_ID);
});

test("plain bodies and message text are read without casts; sealed ones stay unread", () => {
	const reaction = entry("r", {
		kind: "reaction",
		envelope: plainEnvelope({ target: "m_a", emoji: "👍", op: "add" }),
	});
	const sealed = entry("s", {
		envelope: {
			v: 1,
			enc: "hpke",
			key_id: "k",
			enc_key: "e",
			ct: "c",
			body: { text: "not plain" },
		} as InboxEntry["envelope"],
	});
	expect(inboxEntryText(entry("hello"))).toBe("hello");
	expect(inboxEntryText(reaction)).toBeUndefined();
	expect(plainBody<InboxReactionBody>(reaction)?.emoji).toBe("👍");
	expect(inboxEntryText(sealed)).toBeUndefined();
	expect(plainBody(sealed)).toBeUndefined();
});
