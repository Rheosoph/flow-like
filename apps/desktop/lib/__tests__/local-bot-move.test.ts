import type { IBot } from "@flow-like/flow-like-ui";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../components/tauri-provider";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn(),
	get: vi.fn(),
	post: vi.fn(),
	put: vi.fn(),
	patch: vi.fn(),
	del: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../api", () => ({
	get: mocks.get,
	post: mocks.post,
	put: mocks.put,
	patch: mocks.patch,
	del: mocks.del,
}));

import { BotState } from "../../components/tauri-provider/bot-state";
import { InboxState } from "../../components/tauri-provider/inbox-state";
import { LocalStoreError, localBots } from "../local-bots/commands";
import { BotMoveError, LocalBotMover } from "../local-bots/move";
import { localRegistries } from "../local-bots/registry";

const SUB = "user-1";
const profile = { id: "profile-1", hub: "hub.example" };
const auth = {
	isAuthenticated: true,
	isLoading: false,
	user: { access_token: "token", profile: { sub: SUB } },
};

function backend(signedIn = true): TauriBackend {
	return {
		profile,
		auth: signedIn ? auth : undefined,
	} as unknown as TauriBackend;
}

function localBot(id: string, name: string): IBot {
	return {
		id,
		owner_id: "local",
		system: null,
		preset_id: null,
		name,
		description: null,
		instructions: null,
		appearance: { kind: "pip" },
		model: { primary: null, fallbacks: [] },
		classification: { pii: false },
		capabilities: [],
		profile_id: null,
		location: "desktop",
		device_id: null,
		created_at: "2026-10-30T09:00:00Z",
		updated_at: "2026-11-01T09:00:00Z",
		local: true,
	} as unknown as IBot;
}

const REPORT = {
	bot_id: "bot_a",
	issues: [],
	apps: 2,
	secrets: 2,
	conversations: 2,
	messages: 4,
	waiting_conversations: 1,
	skipped_messages: 0,
};

const MOVED = {
	bot: { id: "bot_acct1", owner_id: SUB, name: "Ada" },
	conversations: [{ local_id: "c-direct", account_id: "chat_existing" }],
};

const APP_ISSUE = {
	part: "app",
	code: "BOT_APP_UNAVAILABLE",
	message: "You can't use app app-2",
	ref_id: "app-2",
};

/** The Tauri commands by name; a rejection is thrown as the command rejects. */
let commands: Record<string, (args: Record<string, unknown>) => unknown>;

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	commands = {
		local_bots_list: () => [localBot("bot_a", "Ada")],
		local_bots_get: ({ botId }) => localBot(String(botId), "Ada"),
		local_bots_check_move: () => REPORT,
		local_bots_move: () => MOVED,
	};
	mocks.invoke.mockImplementation(
		async (command: string, args: Record<string, unknown> = {}) => {
			const handle = commands[command];
			if (!handle) throw `Command ${command} not found`;
			return handle(args);
		},
	);
	mocks.get.mockImplementation(async (_profile, path: string) => {
		if (path === "bots") return { bots: [] };
		if (path.endsWith("/entries")) return { entries: [], next_before: null };
		if (path.endsWith("/secrets")) return { secrets: [] };
		return { id: path.split("/")[1], owner_id: SUB };
	});
});

afterEach(() => vi.restoreAllMocks());

function reject(command: string, rejection: unknown) {
	commands[command] = () => {
		throw rejection;
	};
}

function hubWrites() {
	return [mocks.post, mocks.put, mocks.patch, mocks.del].flatMap(
		(mock) => mock.mock.calls,
	);
}

describe("the bots on this computer", () => {
	test("include a built-in the signed-in list leaves out for the account's own", async () => {
		const untouched = {
			...localBot("bot_general_here", "Assistant"),
			system: "general",
			updated_at: "2026-10-30T09:00:00Z",
		};
		commands.local_bots_list = () => [untouched];
		mocks.get.mockImplementation(async (_profile, path: string) =>
			path === "bots"
				? {
						bots: [
							{
								...untouched,
								id: "bot_general_acct",
								owner_id: SUB,
								local: undefined,
							},
						],
					}
				: { id: path.split("/")[1], owner_id: SUB },
		);
		const bots = new BotState(backend());
		expect((await bots.listBots()).map((bot) => bot.id)).toEqual([
			"bot_general_acct",
		]);
		expect((await bots.local.list()).map((bot) => [bot.id, bot.local])).toEqual(
			[["bot_general_here", true]],
		);
	});
});

describe("following a move", () => {
	test("hands the desktop's step events to the listener until it stops", async () => {
		const unlisten = vi.fn();
		let deliver: (event: { payload: unknown }) => void = () => {};
		mocks.listen.mockImplementation(async (_name, handler) => {
			deliver = handler;
			return unlisten;
		});
		const steps: unknown[] = [];
		const stop = await new LocalBotMover(backend()).onProgress((progress) =>
			steps.push(progress),
		);

		expect(mocks.listen).toHaveBeenCalledWith(
			"local-bots://move",
			expect.any(Function),
		);
		deliver({ payload: { bot_id: "bot_a", step: "workspace", pass: 1 } });
		expect(steps).toEqual([{ bot_id: "bot_a", step: "workspace", pass: 1 }]);
		stop();
		expect(unlisten).toHaveBeenCalled();
	});
});

describe("checking a move", () => {
	test("asks this computer with the hub's token and returns its review", async () => {
		const report = await new LocalBotMover(backend()).checkMove("bot_a");

		expect(report).toEqual(REPORT);
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_check_move", {
			botId: "bot_a",
			accessToken: "token",
		});
		expect(mocks.get).not.toHaveBeenCalled();
		expect(hubWrites()).toEqual([]);
	});

	test("needs an account", async () => {
		await expect(
			new LocalBotMover(backend(false)).checkMove("bot_a"),
		).rejects.toThrow("Sign in to move bot bot_a to your account");
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("a refusal keeps its code and step, so a locked keychain reads as one", async () => {
		reject("local_bots_check_move", {
			code: "SECRET_KEY",
			message: "Could not export local bot bot_a: the keychain is locked",
			step: "check",
		});
		const failure = await new LocalBotMover(backend())
			.checkMove("bot_a")
			.catch((error: unknown) => error);

		expect(failure).toBeInstanceOf(BotMoveError);
		expect(failure).toMatchObject({
			code: "SECRET_KEY",
			step: "check",
			message: "Could not export local bot bot_a: the keychain is locked",
			issue: undefined,
		});
	});

	test("an answer that is no review fails", async () => {
		commands.local_bots_check_move = () => null;
		await expect(
			new LocalBotMover(backend()).checkMove("bot_a"),
		).rejects.toThrow(
			"Could not check bot bot_a against your account: this computer returned no review",
		);
	});
});

describe("moving a bot", () => {
	test("returns the account's bot; only the sign-in leaves the webview", async () => {
		const moved = await new LocalBotMover(backend()).move("bot_a");

		expect(moved).toEqual({ bot: MOVED.bot, warnings: 0 });
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_move", {
			botId: "bot_a",
			accessToken: "token",
		});
		expect(hubWrites()).toEqual([]);
	});

	test("reports how many file links and summaries could not move", async () => {
		commands.local_bots_move = () => ({ ...MOVED, warnings: 3 });
		const moved = await new LocalBotMover(backend()).move("bot_a");

		expect(moved.warnings).toBe(3);
		expect(moved.bot).toEqual(MOVED.bot);
	});

	test("needs an account", async () => {
		await expect(
			new LocalBotMover(backend(false)).move("bot_a"),
		).rejects.toThrow("Sign in to move bot bot_a to your account");
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("afterwards the bot's local id stands for the account bot, without probing", async () => {
		const state = new BotState(backend());
		await state.listBots();
		await state.local.move("bot_a");
		mocks.invoke.mockClear();

		await state.getBot("bot_a");
		await state.listBotSecrets("bot_a");

		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.get.mock.calls.slice(-2).map((call) => call[1])).toEqual([
			"bots/bot_acct1",
			"bots/bot_acct1/secrets",
		]);
	});

	test("a moved chat's local id reads the account's chat it went into", async () => {
		const desktop = backend();
		await new LocalBotMover(desktop).move("bot_a");
		mocks.invoke.mockClear();

		await new InboxState(desktop).listEntries("c-direct");

		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.get).toHaveBeenLastCalledWith(
			profile,
			"inbox/conversations/chat_existing/entries",
			auth,
		);
	});

	test("a refusal keeps its code and the issue that blocks the move", async () => {
		reject("local_bots_move", {
			code: "BOT_APP_UNAVAILABLE",
			message:
				"Bot bot_a can't move to your account yet: You can't use app app-2",
			step: "check",
			issue: APP_ISSUE,
		});
		const failure = await new LocalBotMover(backend())
			.move("bot_a")
			.catch((error: unknown) => error);

		expect(failure).toBeInstanceOf(BotMoveError);
		expect((failure as BotMoveError).code).toBe("BOT_APP_UNAVAILABLE");
		expect((failure as BotMoveError).issue).toEqual(APP_ISSUE);
		expect((failure as Error).message).toContain("You can't use app app-2");
	});

	test("chats that moved before the move stopped still point to the account, the bot stays here", async () => {
		const desktop = backend();
		reject("local_bots_move", {
			code: "LOCAL_CHANGED",
			message:
				"Could not finish moving bot bot_a: it kept changing on this computer while it moved. Try again once nothing writes to it.",
			step: "verify",
			conversations: [{ local_id: "c-group", account_id: "chat_acct2" }],
		});
		const failure = await new LocalBotMover(desktop)
			.move("bot_a")
			.catch((error: unknown) => error);

		expect(failure).toMatchObject({ code: "LOCAL_CHANGED", step: "verify" });
		const registries = localRegistries(desktop);
		expect(registries.conversations.accountId("c-group", SUB)).toBe(
			"chat_acct2",
		);
		expect(registries.bots.accountId("bot_a", SUB)).toBe("bot_a");
	});

	test("a plain-text failure reads as text", async () => {
		reject("local_bots_move", "database is locked");
		const failure = await new LocalBotMover(backend())
			.move("bot_a")
			.catch((error: unknown) => error);

		expect(failure).toBeInstanceOf(BotMoveError);
		expect((failure as BotMoveError).code).toBeUndefined();
		expect((failure as Error).message).toBe(
			"Could not move bot bot_a to your account: database is locked",
		);
	});

	test("an answer without a bot fails and moves no id", async () => {
		const desktop = backend();
		commands.local_bots_move = () => ({ conversations: [] });

		await expect(new LocalBotMover(desktop).move("bot_a")).rejects.toThrow(
			"Could not move bot bot_a to your account: this computer returned no bot",
		);
		expect(localRegistries(desktop).bots.accountId("bot_a", SUB)).toBe("bot_a");
	});
});

describe("store failures", () => {
	test("carry the store's code, and plain text still reads", async () => {
		reject("local_bots_apps", {
			code: "NOT_FOUND",
			message: "Bot 'bot_a' not found",
		});
		const coded = await localBots
			.apps("bot_a")
			.catch((error: unknown) => error);
		expect(coded).toBeInstanceOf(LocalStoreError);
		expect((coded as LocalStoreError).code).toBe("NOT_FOUND");
		expect((coded as Error).message).toBe(
			"Could not list the apps of local bot bot_a: Bot 'bot_a' not found",
		);

		reject("local_bots_apps", "database is locked");
		const plain = await localBots
			.apps("bot_a")
			.catch((error: unknown) => error);
		expect((plain as LocalStoreError).code).toBeUndefined();
		expect((plain as Error).message).toBe(
			"Could not list the apps of local bot bot_a: database is locked",
		);
	});
});
