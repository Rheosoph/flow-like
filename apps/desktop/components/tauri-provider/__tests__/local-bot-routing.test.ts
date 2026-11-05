import type { IBot, IConversation } from "@flow-like/flow-like-ui";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	get: vi.fn(),
	post: vi.fn(),
	put: vi.fn(),
	patch: vi.fn(),
	del: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../../lib/api", () => ({
	get: mocks.get,
	post: mocks.post,
	put: mocks.put,
	patch: mocks.patch,
	del: mocks.del,
}));

import { localRegistries } from "../../../lib/local-bots/registry";
import { BotState } from "../bot-state";
import { InboxState } from "../inbox-state";

type Handler = (args: Record<string, unknown>) => unknown;

const profile = { id: "profile-1", hub: "hub.example" };
const signedInAuth = {
	isAuthenticated: true,
	isLoading: false,
	user: { access_token: "token", profile: { sub: "user-1" } },
};

function signedIn(): TauriBackend {
	return { profile, auth: signedInAuth } as unknown as TauriBackend;
}

function signedOut(): TauriBackend {
	return { profile, auth: undefined } as unknown as TauriBackend;
}

function bot(id: string, local = false): IBot {
	const fixture = {
		id,
		owner_id: local ? "local" : "user-1",
		name: id,
		location: "desktop",
		...(local ? { local: true } : {}),
	};
	return fixture as unknown as IBot;
}

function conversation(
	id: string,
	lastActivity: string,
	members: string[],
	local = false,
): IConversation {
	const fixture = {
		id,
		kind: members.length === 2 ? "direct" : "group",
		last_activity_at: lastActivity,
		members: members.map((principal_id) => ({ principal_id })),
		...(local ? { local: true } : {}),
	};
	return fixture as unknown as IConversation;
}

function localStore(handlers: Record<string, Handler>) {
	mocks.invoke.mockImplementation(
		async (command: string, args?: Record<string, unknown>) => {
			const handler = handlers[command];
			if (!handler) throw `Command ${command} not found`;
			return handler(args ?? {});
		},
	);
}

function commands(): string[] {
	return mocks.invoke.mock.calls.map(([command]) => command as string);
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	vi.spyOn(console, "warn").mockImplementation(() => undefined);
});

afterEach(() => vi.restoreAllMocks());

describe("bots signed out", () => {
	test("every call goes to the bots on this computer", async () => {
		const local = bot("bot_local", true);
		localStore({
			local_bots_list: () => [local],
			local_bots_get: () => local,
			local_bots_create: () => local,
			local_bots_update: () => local,
			local_bots_delete: () => null,
			local_bots_apps: () => [{ app_id: "app-1" }],
			local_bots_attach_app: () => null,
			local_bots_secrets: () => [],
			local_bots_set_secret: () => ({ name: "API_KEY" }),
			local_bots_delete_secret: () => null,
		});
		const state = new BotState(signedOut());

		expect(await state.listBots()).toEqual([local]);
		expect(await state.getBot("bot_local")).toEqual(local);
		await state.createBot({ name: "Ada", appearance: { kind: "pip" } });
		await state.updateBot("bot_local", { name: "Ada" });
		await state.attachBotApp("bot_local", "app-1");
		await state.listBotApps("bot_local");
		await state.upsertBotSecret("bot_local", "API_KEY", { value: "secret" });
		await state.deleteBotSecret("bot_local", "API_KEY");
		await state.deleteBot("bot_local");

		expect(commands()).toEqual([
			"local_bots_list",
			"local_bots_get",
			"local_bots_create",
			"local_bots_update",
			"local_bots_attach_app",
			"local_bots_apps",
			"local_bots_set_secret",
			"local_bots_delete_secret",
			"local_bots_delete",
		]);
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_create", {
			request: { name: "Ada", appearance: { kind: "pip" } },
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_attach_app", {
			botId: "bot_local",
			appId: "app-1",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_set_secret", {
			botId: "bot_local",
			name: "API_KEY",
			value: "secret",
		});
		for (const call of [mocks.get, mocks.post, mocks.put, mocks.patch]) {
			expect(call).not.toHaveBeenCalled();
		}
	});

	test("a bot the local store doesn't hold is an error naming its id", async () => {
		localStore({ local_bots_get: () => null });
		await expect(new BotState(signedOut()).getBot("bot_x")).rejects.toThrow(
			"Bot bot_x was not found on this computer",
		);
	});

	test("local command failures name the operation and the bot, with the store's code", async () => {
		localStore({
			local_bots_update: () => {
				throw { code: "INVALID", message: "Name is too long" };
			},
			local_bots_delete: () => {
				throw "database is locked";
			},
		});
		const state = new BotState(signedOut());

		const coded = await state
			.updateBot("bot_x", { name: "x" })
			.catch((error: unknown) => error);
		expect((coded as Error).message).toBe(
			"Could not update local bot bot_x: Name is too long",
		);
		expect((coded as { code?: string }).code).toBe("INVALID");

		const plain = await state
			.deleteBot("bot_x")
			.catch((error: unknown) => error);
		expect((plain as Error).message).toBe(
			"Could not delete local bot bot_x: database is locked",
		);
		expect((plain as { code?: string }).code).toBeUndefined();
	});

	test("a sealed secret can't be stored for a local bot", async () => {
		localStore({});
		await expect(
			new BotState(signedOut()).upsertBotSecret("bot_x", "API_KEY", {
				sealed: { v: 1 },
			}),
		).rejects.toThrow("Could not save secret API_KEY of bot bot_x");
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("results are marked local even when the store leaves the flag out", async () => {
		localStore({ local_bots_list: () => [bot("bot_plain")] });
		const [listed] = await new BotState(signedOut()).listBots();
		expect(listed.local).toBe(true);
	});
});

describe("bots signed in", () => {
	test("lists the account's bots in their order, then this computer's", async () => {
		mocks.get.mockResolvedValue({
			bots: [bot("bot_general"), bot("bot_b"), bot("bot_mid_move")],
		});
		localStore({
			local_bots_list: () => [
				bot("bot_local_new", true),
				bot("bot_mid_move", true),
			],
		});

		const bots = await new BotState(signedIn()).listBots();

		expect(bots.map((entry) => [entry.id, entry.local ?? false])).toEqual([
			["bot_general", false],
			["bot_b", false],
			["bot_local_new", true],
			["bot_mid_move", true],
		]);
		expect(mocks.get).toHaveBeenCalledWith(profile, "bots", signedInAuth);
	});

	test("a failing local store still shows the account's bots", async () => {
		mocks.get.mockResolvedValue({ bots: [bot("bot_b")] });
		localStore({});
		expect(await new BotState(signedIn()).listBots()).toEqual([bot("bot_b")]);
	});

	test("an unreachable hub keeps the account's last list beside fresh local bots", async () => {
		const local = [bot("bot_local", true)];
		localStore({ local_bots_list: () => [...local] });
		const state = new BotState(signedIn());
		const offline = new Error("Network unavailable: GET /bots");

		mocks.get.mockRejectedValueOnce(offline);
		await expect(state.listBots()).rejects.toBe(offline);

		mocks.get.mockResolvedValueOnce({ bots: [bot("bot_b")] });
		await state.listBots();
		local.push(bot("bot_new", true));
		mocks.get.mockRejectedValueOnce(offline);
		expect((await state.listBots()).map((entry) => entry.id)).toEqual([
			"bot_b",
			"bot_local",
			"bot_new",
		]);

		const refusal = Object.assign(new Error("[UNAUTHORIZED] Sign in"), {
			status: 401,
		});
		mocks.get.mockRejectedValueOnce(refusal);
		await expect(state.listBots()).rejects.toBe(refusal);
	});

	test("new bots go to the account", async () => {
		mocks.post.mockResolvedValue(bot("bot_new"));
		localStore({});
		const request = { name: "Ada", appearance: { kind: "pip" } };
		await new BotState(signedIn()).createBot(request);
		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"bots",
			request,
			signedInAuth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("listed ids route without probing", async () => {
		mocks.get.mockResolvedValue({ bots: [bot("bot_remote")] });
		mocks.patch.mockResolvedValue(bot("bot_remote"));
		localStore({
			local_bots_list: () => [bot("bot_local", true)],
			local_bots_update: () => bot("bot_local", true),
		});
		const state = new BotState(signedIn());
		await state.listBots();
		mocks.invoke.mockClear();

		await state.updateBot("bot_local", { name: "Local" });
		await state.updateBot("bot_remote", { name: "Remote" });

		expect(commands()).toEqual(["local_bots_update"]);
		expect(mocks.patch).toHaveBeenCalledTimes(1);
		expect(mocks.patch.mock.calls[0][1]).toBe("bots/bot_remote");
	});

	test("an unknown id is probed once, then routed where it lives", async () => {
		mocks.get.mockResolvedValue({ apps: [{ app_id: "app-remote" }] });
		localStore({
			local_bots_get: ({ botId }) =>
				botId === "bot_here" ? bot("bot_here", true) : null,
			local_bots_apps: () => [{ app_id: "app-local" }],
		});
		const state = new BotState(signedIn());

		expect(await state.listBotApps("bot_here")).toEqual([
			{ app_id: "app-local" },
		]);
		expect(await state.listBotApps("bot_there")).toEqual([
			{ app_id: "app-remote" },
		]);
		await state.listBotApps("bot_here");
		await state.listBotApps("bot_there");

		expect(commands()).toEqual([
			"local_bots_get",
			"local_bots_apps",
			"local_bots_get",
			"local_bots_apps",
		]);
		expect(mocks.get.mock.calls.map((call) => call[1])).toEqual([
			"bots/bot_there/apps",
			"bots/bot_there/apps",
		]);
	});

	test("a probe the store can't answer fails the call instead of asking the account, and isn't remembered", async () => {
		mocks.get.mockResolvedValue({ apps: [] });
		let locked = true;
		localStore({
			local_bots_get: () => {
				if (locked) throw { code: "STORAGE", message: "database is locked" };
				return bot("bot_x", true);
			},
			local_bots_apps: () => [{ app_id: "app-local" }],
		});
		const state = new BotState(signedIn());

		await expect(state.listBotApps("bot_x")).rejects.toThrow(
			"Could not load local bot bot_x: database is locked",
		);
		await expect(
			state.upsertBotSecret("bot_x", "API_KEY", { value: "s3cret" }),
		).rejects.toThrow("database is locked");
		expect(mocks.get).not.toHaveBeenCalled();
		expect(mocks.put).not.toHaveBeenCalled();

		locked = false;
		expect(await state.listBotApps("bot_x")).toEqual([{ app_id: "app-local" }]);
		expect(mocks.get).not.toHaveBeenCalled();
	});

	test("a bot the store no longer holds loses its local mark, so the next call asks again", async () => {
		localStore({
			local_bots_list: () => [bot("bot_gone", true)],
			local_bots_update: () => {
				throw { code: "NOT_FOUND", message: "Bot 'bot_gone' not found" };
			},
			local_bots_get: () => null,
		});
		mocks.get.mockResolvedValue({ bots: [] });
		mocks.patch.mockResolvedValue(bot("bot_gone"));
		const state = new BotState(signedIn());
		await state.listBots();

		const failure = await state
			.updateBot("bot_gone", { name: "Gone" })
			.catch((error: unknown) => error);
		expect((failure as { code?: string }).code).toBe("NOT_FOUND");
		await state.updateBot("bot_gone", { name: "Gone" });

		expect(commands()).toEqual([
			"local_bots_list",
			"local_bots_update",
			"local_bots_get",
		]);
		expect(mocks.patch.mock.calls.map((call) => call[1])).toEqual([
			"bots/bot_gone",
		]);
	});

	test("a moved bot's local id stands for its account id, even beside an older listing", async () => {
		const backend = signedIn();
		localStore({ local_bots_list: () => [bot("bot_old", true)] });
		mocks.get.mockImplementation(async (_profile, path: string) =>
			path === "bots" ? { bots: [bot("bot_new")] } : bot("bot_new"),
		);
		localRegistries(backend).bots.moveToAccount("user-1", "bot_old", "bot_new");
		const state = new BotState(backend);
		await state.listBots();
		mocks.invoke.mockClear();

		await state.getBot("bot_old");
		await state.attachBotApp("bot_old", "app-1");

		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.get).toHaveBeenLastCalledWith(
			profile,
			"bots/bot_new",
			signedInAuth,
		);
		expect(mocks.put).toHaveBeenCalledWith(
			profile,
			"bots/bot_new/apps/app-1",
			undefined,
			signedInAuth,
		);
	});

	test("deleting a local bot forgets it, so the next call asks again", async () => {
		localStore({
			local_bots_list: () => [bot("bot_local", true)],
			local_bots_delete: () => null,
			local_bots_get: () => null,
		});
		mocks.get.mockImplementation(async (_profile, path: string) =>
			path === "bots" ? { bots: [] } : bot("bot_local"),
		);
		const state = new BotState(signedIn());
		await state.listBots();
		await state.deleteBot("bot_local");
		await state.getBot("bot_local");
		expect(commands()).toEqual([
			"local_bots_list",
			"local_bots_delete",
			"local_bots_get",
		]);
	});
});

describe("inbox signed out", () => {
	test("conversations, entries and reads stay on this computer", async () => {
		const local = conversation(
			"c-local",
			"2026-11-02T08:00:00Z",
			["local", "bot_a"],
			true,
		);
		localStore({
			local_inbox_conversations: () => [local],
			local_inbox_conversation: () => local,
			local_inbox_create: () => local,
			local_inbox_entries: () => ({ entries: [], next_before: null }),
			local_inbox_send: () => ({ id: "entry" }),
			local_inbox_mark_read: () => ({ last_read_at: "now" }),
		});
		const state = new InboxState(signedOut());

		expect(await state.listConversations()).toEqual([local]);
		await state.getConversation("c-local", "bot_a");
		await state.createConversation({ kind: "direct", members: ["bot_a"] });
		await state.listEntries("c-local", {
			as: "bot_a",
			before: "1:a",
			limit: 20,
		});
		await state.sendEntry("c-local", { message_id: "m1", kind: "message" });
		await state.markRead("c-local", "1:a");

		expect(mocks.invoke).toHaveBeenCalledWith("local_inbox_conversation", {
			conversationId: "c-local",
			principalId: "bot_a",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_inbox_entries", {
			conversationId: "c-local",
			principalId: "bot_a",
			before: "1:a",
			limit: 20,
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_inbox_mark_read", {
			conversationId: "c-local",
			upTo: "1:a",
		});
		expect(mocks.get).not.toHaveBeenCalled();
		expect(mocks.post).not.toHaveBeenCalled();
	});

	test("reading as a bot shows the local conversations it is in", async () => {
		localStore({
			local_inbox_conversations: () => [
				conversation("c-a", "2026-11-02T08:00:00Z", ["local", "bot_a"]),
				conversation("c-b", "2026-11-02T07:00:00Z", ["local", "bot_b"]),
			],
		});
		const listed = await new InboxState(signedOut()).listConversations("bot_b");
		expect(listed.map((entry) => entry.id)).toEqual(["c-b"]);
	});

	test("a missing local chat has a code that lets an open view clear stale history", async () => {
		localStore({ local_inbox_conversation: () => null });
		await expect(
			new InboxState(signedOut()).getConversation("c-gone"),
		).rejects.toMatchObject({
			name: "LocalStoreError",
			code: "NOT_FOUND",
		});
	});

	test("inbox keys need an account", async () => {
		localStore({});
		const state = new InboxState(signedOut());
		expect(await state.getKeys(["local", "bot_a"])).toEqual([]);
		await expect(
			state.registerKey({ key_id: "k1", algorithm: "x", public_key: "p" }),
		).rejects.toThrow(
			"Could not register inbox key k1: inbox keys need an account",
		);
		await expect(state.revokeKey("k1")).rejects.toThrow(
			"Could not revoke inbox key k1: inbox keys need an account",
		);
		expect(mocks.get).not.toHaveBeenCalled();
	});
});

describe("inbox signed in", () => {
	test("merges the account's and this computer's conversations by newest activity", async () => {
		mocks.get.mockResolvedValue({
			conversations: [
				conversation("c-remote-new", "2026-11-02T09:00:00Z", [
					"user-1",
					"bot_b",
				]),
				conversation("c-remote-old", "2026-11-01T09:00:00Z", [
					"user-1",
					"bot_b",
				]),
				conversation("c-mid-move", "2026-11-02T08:30:00Z", ["user-1", "bot_a"]),
			],
		});
		localStore({
			local_inbox_conversations: () => [
				conversation(
					"c-local",
					"2026-11-02T08:45:00.123456+00:00",
					["local", "bot_a"],
					true,
				),
				conversation(
					"c-mid-move",
					"2026-11-02T08:30:00Z",
					["local", "bot_a"],
					true,
				),
			],
		});

		const listed = await new InboxState(signedIn()).listConversations();

		expect(listed.map((entry) => [entry.id, entry.local ?? false])).toEqual([
			["c-remote-new", false],
			["c-local", true],
			["c-mid-move", true],
			["c-remote-old", false],
		]);
	});

	test("an unreachable hub keeps the account's last conversations while local ones update", async () => {
		let localActivity = "2026-11-02T08:00:00Z";
		localStore({
			local_inbox_conversations: () => [
				conversation("c-local", localActivity, ["local", "bot_a"], true),
			],
		});
		const state = new InboxState(signedIn());
		mocks.get.mockResolvedValueOnce({
			conversations: [
				conversation("c-remote", "2026-11-02T09:00:00Z", ["user-1", "bot_b"]),
			],
		});
		await state.listConversations();

		localActivity = "2026-11-02T10:00:00Z";
		mocks.get.mockRejectedValueOnce(
			Object.assign(new Error("[UPSTREAM_UNAVAILABLE] Try again"), {
				status: 503,
			}),
		);
		const listed = await state.listConversations();

		expect(listed.map((entry) => entry.id)).toEqual(["c-local", "c-remote"]);
	});

	test("deleting a local conversation stays local even while signed in", async () => {
		localStore({
			local_inbox_conversation: () =>
				conversation("c-local", "2026-11-02T08:00:00Z", ["local", "bot_a"]),
			local_inbox_delete_conversation: () => null,
		});
		await new InboxState(signedIn()).deleteConversation("c-local");
		expect(mocks.invoke).toHaveBeenCalledWith(
			"local_inbox_delete_conversation",
			{
				conversationId: "c-local",
			},
		);
		expect(mocks.del).not.toHaveBeenCalled();
	});

	test("deleted account chats stay out of the offline fallback after a stale read", async () => {
		const backend = signedIn();
		localStore({ local_inbox_conversations: () => [] });
		const gone = conversation("c-remote", "2026-11-02T09:00:00Z", [
			"user-1",
			"bot_b",
		]);
		const kept = conversation("c-kept", "2026-11-02T08:00:00Z", [
			"user-1",
			"bot_c",
		]);
		mocks.get.mockResolvedValueOnce({ conversations: [gone, kept] });
		const state = new InboxState(backend);
		await state.listConversations();
		let complete!: (value: { conversations: IConversation[] }) => void;
		mocks.get.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					complete = resolve;
				}),
		);
		const stale = state.listConversations();
		await state.deleteConversation(gone.id);
		complete({ conversations: [gone, kept] });
		expect((await stale).map((entry) => entry.id)).toEqual([kept.id]);
		mocks.get.mockRejectedValueOnce(
			Object.assign(new Error("[UPSTREAM_UNAVAILABLE] Try again"), {
				status: 503,
			}),
		);
		expect((await state.listConversations()).map((entry) => entry.id)).toEqual([
			kept.id,
		]);
		expect(mocks.del).toHaveBeenCalledWith(
			profile,
			"inbox/conversations/c-remote",
			undefined,
			signedInAuth,
		);
	});

	test("deletion follows a moved chat's account id and preserves failures", async () => {
		const backend = signedIn();
		localRegistries(backend).conversations.moveToAccount(
			"user-1",
			"c-old",
			"c/new",
		);
		const state = new InboxState(backend);
		mocks.del.mockRejectedValueOnce(new Error("offline"));
		await expect(state.deleteConversation("c-old")).rejects.toThrow("offline");
		expect(mocks.del).toHaveBeenCalledWith(
			profile,
			"inbox/conversations/c%2Fnew",
			undefined,
			signedInAuth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("a conversation with only local bots stays local; others go to the account", async () => {
		localStore({
			local_bots_get: ({ botId }) =>
				String(botId).startsWith("bot_local") ? bot(String(botId), true) : null,
			local_inbox_create: () =>
				conversation("c-local", "2026-11-02T08:00:00Z", [
					"local",
					"bot_local_a",
				]),
		});
		mocks.post.mockResolvedValue(
			conversation("c-remote", "2026-11-02T08:00:00Z", ["user-1", "bot_b"]),
		);
		const state = new InboxState(signedIn());

		await state.createConversation({
			kind: "group",
			title: "Locals",
			members: ["bot_local_a", "bot_local_b"],
		});
		await state.createConversation({ kind: "direct", members: ["bot_b"] });
		await expect(
			state.createConversation({
				kind: "group",
				members: ["bot_local_a", "bot_b"],
			}),
		).rejects.toThrow(
			"Could not create a group conversation: bot_local_a live only on this computer",
		);

		expect(mocks.invoke).toHaveBeenCalledWith("local_inbox_create", {
			request: {
				kind: "group",
				title: "Locals",
				members: ["bot_local_a", "bot_local_b"],
			},
		});
		expect(mocks.post).toHaveBeenCalledTimes(1);
		expect(mocks.post.mock.calls[0][1]).toBe("inbox/conversations");
	});

	test("an unknown conversation is probed; null means it lives in the account", async () => {
		localStore({
			local_inbox_conversation: ({ conversationId }) =>
				conversationId === "c-local"
					? conversation("c-local", "2026-11-02T08:00:00Z", ["local", "bot_a"])
					: null,
			local_inbox_send: () => ({ id: "entry-local" }),
		});
		mocks.post.mockResolvedValue({ id: "entry-remote" });
		const state = new InboxState(signedIn());
		const request = { message_id: "m1", kind: "message" as const };

		expect(await state.sendEntry("c-local", request)).toEqual({
			id: "entry-local",
		});
		expect(await state.sendEntry("c-remote", request)).toEqual({
			id: "entry-remote",
		});
		await state.sendEntry("c-remote", request);

		expect(commands()).toEqual([
			"local_inbox_conversation",
			"local_inbox_send",
			"local_inbox_conversation",
		]);
		expect(mocks.post.mock.calls.map((call) => call[1])).toEqual([
			"inbox/conversations/c-remote/entries",
			"inbox/conversations/c-remote/entries",
		]);
	});

	test("a conversation probe the store can't answer keeps the message off the account", async () => {
		localStore({
			local_inbox_conversation: () => {
				throw { code: "STORAGE", message: "disk I/O error" };
			},
		});
		const state = new InboxState(signedIn());

		await expect(
			state.sendEntry("c-unknown", { message_id: "m1", kind: "message" }),
		).rejects.toThrow(
			"Could not load local conversation c-unknown: disk I/O error",
		);
		expect(mocks.post).not.toHaveBeenCalled();
	});

	test("a moved conversation's local id stands for its account id", async () => {
		const backend = signedIn();
		localStore({
			local_inbox_conversations: () => [
				conversation("c-old", "2026-11-02T08:00:00Z", ["local", "bot_a"], true),
			],
		});
		mocks.get.mockResolvedValue({ conversations: [] });
		mocks.post.mockResolvedValue({ id: "entry" });
		localRegistries(backend).conversations.moveToAccount(
			"user-1",
			"c-old",
			"c-new",
		);
		const state = new InboxState(backend);
		await state.listConversations();
		mocks.invoke.mockClear();

		const request = { message_id: "m1", kind: "message" as const };
		await state.sendEntry("c-old", request);

		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"inbox/conversations/c-new/entries",
			request,
			signedInAuth,
		);
	});

	test("a conversation the store no longer holds loses its local mark", async () => {
		localStore({
			local_inbox_conversations: () => [
				conversation(
					"c-gone",
					"2026-11-02T08:00:00Z",
					["local", "bot_a"],
					true,
				),
			],
			local_inbox_send: () => {
				throw { code: "NOT_FOUND", message: "Conversation 'c-gone' not found" };
			},
			local_inbox_conversation: () => null,
		});
		mocks.get.mockResolvedValue({ conversations: [] });
		mocks.post.mockResolvedValue({ id: "entry-remote" });
		const state = new InboxState(signedIn());
		await state.listConversations();
		const request = { message_id: "m1", kind: "message" as const };

		await expect(state.sendEntry("c-gone", request)).rejects.toThrow(
			"not found",
		);
		expect(await state.sendEntry("c-gone", request)).toEqual({
			id: "entry-remote",
		});
		expect(commands()).toEqual([
			"local_inbox_conversations",
			"local_inbox_send",
			"local_inbox_conversation",
		]);
	});

	test("leaving a local conversation removes `local`, not the account's sub", async () => {
		localStore({
			local_inbox_conversations: () => [
				conversation(
					"c-local",
					"2026-11-02T08:00:00Z",
					["local", "bot_a"],
					true,
				),
			],
			local_inbox_remove_member: () => null,
		});
		mocks.get.mockResolvedValue({ conversations: [] });
		const state = new InboxState(signedIn());
		await state.listConversations();
		await state.removeMember("c-local", "user-1");
		await state.removeMember("c-local", "bot_a");
		expect(mocks.invoke).toHaveBeenCalledWith("local_inbox_remove_member", {
			conversationId: "c-local",
			principalId: "local",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_inbox_remove_member", {
			conversationId: "c-local",
			principalId: "bot_a",
		});
	});

	test("keys are fetched only for account principals", async () => {
		localStore({
			local_bots_get: ({ botId }) =>
				botId === "bot_local" ? bot("bot_local", true) : null,
		});
		mocks.get.mockResolvedValue({ keys: [{ key_id: "k-user" }] });
		const state = new InboxState(signedIn());

		expect(await state.getKeys(["local", "bot_local"])).toEqual([]);
		expect(mocks.get).not.toHaveBeenCalled();
		expect(await state.getKeys(["user-2", "bot_local", "bot_b"])).toEqual([
			{ key_id: "k-user" },
		]);
		expect(mocks.get).toHaveBeenCalledWith(
			profile,
			"inbox/keys?principals=user-2%2Cbot_b",
			signedInAuth,
		);
	});

	test("keys of principals the store can't place are still asked of the account", async () => {
		localStore({
			local_bots_get: () => {
				throw { code: "STORAGE", message: "database is locked" };
			},
		});
		mocks.get.mockResolvedValue({ keys: [{ key_id: "k-user" }] });

		expect(await new InboxState(signedIn()).getKeys(["user-2"])).toEqual([
			{ key_id: "k-user" },
		]);
		expect(mocks.get).toHaveBeenCalledWith(
			profile,
			"inbox/keys?principals=user-2",
			signedInAuth,
		);
	});

	test("a local bot can't register or revoke inbox keys", async () => {
		localStore({ local_bots_get: () => bot("bot_local", true) });
		const state = new InboxState(signedIn());
		await expect(
			state.registerKey({
				principal_id: "bot_local",
				key_id: "k1",
				algorithm: "x",
				public_key: "p",
			}),
		).rejects.toThrow(
			"Could not register inbox key k1: bot_local lives only on this computer",
		);
		await expect(state.revokeKey("k1", "bot_local")).rejects.toThrow(
			"Could not revoke inbox key k1: bot_local lives only on this computer",
		);
		expect(mocks.put).not.toHaveBeenCalled();
		expect(mocks.del).not.toHaveBeenCalled();
	});

	test("local bots can't be added to an account conversation", async () => {
		localStore({
			local_inbox_conversation: () => null,
			local_bots_get: ({ botId }) =>
				botId === "bot_local" ? bot("bot_local", true) : null,
		});
		const state = new InboxState(signedIn());
		await expect(
			state.addMembers("c-remote", ["bot_b", "bot_local"]),
		).rejects.toThrow(
			"Could not add members to conversation c-remote: bot_local live only on this computer",
		);
		expect(mocks.post).not.toHaveBeenCalled();
	});
});
