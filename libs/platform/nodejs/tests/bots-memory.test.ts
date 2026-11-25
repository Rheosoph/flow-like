import { afterEach, describe, expect, spyOn, test } from "bun:test";
import {
	type BotMemoryEntity,
	type BotMemoryFact,
	FlowLikeClient,
	FlowLikeError,
	NotFoundError,
} from "../src/index.js";

const BASE = "https://flow.test/api/v1";

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

const fact: BotMemoryFact = {
	id: "f_1",
	text: "Anna lives in Berlin",
	kind: "fact",
	subject: "Anna",
	subject_id: "e_anna",
	source: "chat",
	confidence: 0.9,
	recorded_at: "2026-11-02T10:00:00.000Z",
	valid_from: null,
	valid_to: null,
	superseded_by: null,
	evidence: [
		{
			seq: 3,
			message_id: "msg_1",
			quote: "I live in Berlin",
			at: "2026-11-02T10:00:00.000Z",
			speaker: { id: "user_1", name: "Anna" },
		},
	],
	conversation_scope: "conv_1",
};

const entity: BotMemoryEntity = {
	id: "e_anna",
	name: "Anna",
	kind: "person",
	owner: false,
	aliases: ["Ann"],
	fact_count: 2,
	created_at: "2026-11-02T10:00:00.000Z",
	updated_at: "2026-11-02T10:00:00.000Z",
};

describe("bot memory", () => {
	test("listBotMemoryFacts unwraps the list and sends include_superseded only when asked", async () => {
		const mock = transport(json({ facts: [fact] }), json({ facts: [] }));
		const sdk = client();
		expect(await sdk.listBotMemoryFacts("bot_1")).toEqual([fact]);
		await sdk.listBotMemoryFacts("bot_1", { includeSuperseded: true });
		expect(sent(mock, 0)).toMatchObject({
			method: "GET",
			url: `${BASE}/bots/bot_1/memory/facts`,
		});
		expect(sent(mock, 0).headers.get("authorization")).toBe("pat_owner");
		expect(sent(mock, 1).url).toBe(
			`${BASE}/bots/bot_1/memory/facts?include_superseded=true`,
		);
	});

	test("forgetBotMemoryFact escapes ids and returns the forgotten fact", async () => {
		const mock = transport(json(fact));
		expect(await client().forgetBotMemoryFact("bot/1", "f 1")).toEqual(fact);
		expect(sent(mock)).toMatchObject({
			method: "DELETE",
			url: `${BASE}/bots/bot%2F1/memory/facts/f%201`,
		});
	});

	test("resetBotMemory deletes the whole memory", async () => {
		const mock = transport(json({ status: "ok" }));
		await client().resetBotMemory("bot_1");
		expect(sent(mock)).toMatchObject({
			method: "DELETE",
			url: `${BASE}/bots/bot_1/memory`,
		});
	});

	test("listBotMemoryEntities unwraps the list", async () => {
		const mock = transport(json({ entities: [entity] }));
		expect(await client().listBotMemoryEntities("bot_1")).toEqual([entity]);
		expect(sent(mock)).toMatchObject({
			method: "GET",
			url: `${BASE}/bots/bot_1/memory/entities`,
		});
	});

	test("updateBotMemoryEntity sends the changes and the encoded scope", async () => {
		const mock = transport(json(entity), json(entity));
		const sdk = client();
		await sdk.updateBotMemoryEntity("bot_1", "e_anna", { name: "Anna B." });
		await sdk.updateBotMemoryEntity(
			"bot_1",
			"e_anna",
			{ kind: "organization" },
			{ scope: "room/1 a&b" },
		);
		expect(sent(mock, 0)).toMatchObject({
			method: "PATCH",
			url: `${BASE}/bots/bot_1/memory/entities/e_anna`,
			body: { name: "Anna B." },
		});
		expect(sent(mock, 1)).toMatchObject({
			method: "PATCH",
			url: `${BASE}/bots/bot_1/memory/entities/e_anna?scope=room%2F1+a%26b`,
			body: { kind: "organization" },
		});
	});

	test("mergeBotMemoryEntity posts the target as into", async () => {
		const mock = transport(json(entity));
		await client().mergeBotMemoryEntity("bot_1", "e_ann", "e_anna", {
			scope: "conv_1",
		});
		expect(sent(mock)).toMatchObject({
			method: "POST",
			url: `${BASE}/bots/bot_1/memory/entities/e_ann/merge?scope=conv_1`,
			body: { into: "e_anna" },
		});
	});

	test("forgetBotMemoryEntity returns the forgotten facts", async () => {
		const mock = transport(json({ forgotten: [fact] }));
		const forgotten = await client().forgetBotMemoryEntity("bot_1", "e_anna", {
			scope: "conv_1",
		});
		expect(forgotten).toEqual([fact]);
		expect(sent(mock)).toMatchObject({
			method: "DELETE",
			url: `${BASE}/bots/bot_1/memory/entities/e_anna?scope=conv_1`,
		});
	});

	test("busy memory, refusals and missing entities surface as typed errors", async () => {
		transport(
			json({ error: { code: "MEMORY_BUSY", message: "Memory is busy" } }, 409),
			json({ error: { code: "BAD_REQUEST", message: "Owner entity" } }, 400),
			json({ error: { code: "NOT_FOUND", message: "No such entity" } }, 404),
		);
		const sdk = client();
		const busy = await sdk.resetBotMemory("bot_1").catch((e) => e);
		expect(busy).toBeInstanceOf(FlowLikeError);
		expect(busy.statusCode).toBe(409);
		expect(busy.message).toBe("Memory is busy");
		const refused = await sdk
			.mergeBotMemoryEntity("bot_1", "e_user", "e_anna")
			.catch((e) => e);
		expect(refused.statusCode).toBe(400);
		const missing = await sdk
			.forgetBotMemoryEntity("bot_1", "e_nobody")
			.catch((e) => e);
		expect(missing).toBeInstanceOf(NotFoundError);
	});
});
