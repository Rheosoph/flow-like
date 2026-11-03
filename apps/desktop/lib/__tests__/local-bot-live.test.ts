import type { IBot } from "@flow-like/flow-like-ui";
import { beforeEach, describe, expect, test, vi } from "vitest";
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

function bot(id: string, local: boolean): IBot {
	return {
		id,
		owner_id: local ? "local" : SUB,
		system: null,
		name: id,
		location: "desktop",
		capabilities: [],
		...(local ? { local: true } : {}),
	} as unknown as IBot;
}

function grant(from: string, to: string) {
	return {
		id: `${from}>${to}`,
		from_bot_id: from,
		to_bot_id: to,
		granted_at: "2026-11-03T10:00:00Z",
		granted_by: SUB,
	};
}

const LOCAL_GRANTS = [
	grant("local_a", "local_b"),
	grant("local_c", "local_a"),
	grant("local_b", "local_c"),
];

async function signedInState() {
	const state = new BotState(backend());
	mocks.get.mockImplementation(async (_profile, path: string) =>
		path === "bots"
			? { bots: [bot("acct_a", false), bot("acct_b", false)] }
			: {},
	);
	mocks.invoke.mockImplementation(async (command: string) =>
		command === "local_bots_list" ? [bot("local_a", true)] : null,
	);
	await state.listBots();
	mocks.get.mockReset();
	mocks.invoke.mockReset();
	return state;
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
});

describe("what account bots may ask", () => {
	test("reads the bot's grants from the hub", async () => {
		const state = await signedInState();
		mocks.get.mockResolvedValue({
			granted: [grant("acct_a", "acct_b")],
			received: [],
		});

		await expect(state.listPermissions("acct_a")).resolves.toEqual({
			granted: [grant("acct_a", "acct_b")],
			received: [],
		});
		expect(mocks.get).toHaveBeenCalledWith(
			profile,
			"bots/acct_a/permissions",
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("grants and revokes by the asking bot and the asked bot", async () => {
		const state = await signedInState();
		mocks.put.mockResolvedValue(grant("acct_a", "acct_b"));

		await state.grantPermission("acct_a", "acct_b");
		await state.revokePermission("acct_a", "acct_b");

		expect(mocks.put).toHaveBeenCalledWith(
			profile,
			"bots/acct_a/permissions/acct_b",
			undefined,
			auth,
		);
		expect(mocks.del).toHaveBeenCalledWith(
			profile,
			"bots/acct_a/permissions/acct_b",
			undefined,
			auth,
		);
	});
});

describe("what local bots may ask", () => {
	test("splits this computer's grants into the ones given and received", async () => {
		const state = await signedInState();
		mocks.invoke.mockResolvedValue(LOCAL_GRANTS);

		await expect(state.listPermissions("local_a")).resolves.toEqual({
			granted: [LOCAL_GRANTS[0]],
			received: [LOCAL_GRANTS[1]],
		});
		expect(mocks.invoke).toHaveBeenCalledWith(
			"local_bots_permissions",
			undefined,
		);
		expect(mocks.get).not.toHaveBeenCalled();
	});

	test("grants and revokes through the desktop's commands", async () => {
		const state = await signedInState();
		mocks.invoke.mockResolvedValue(grant("local_a", "local_b"));

		await state.grantPermission("local_a", "local_b");
		await state.revokePermission("local_a", "local_b");

		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_grant_permission", {
			fromBotId: "local_a",
			toBotId: "local_b",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_revoke_permission", {
			fromBotId: "local_a",
			toBotId: "local_b",
		});
		expect(mocks.put).not.toHaveBeenCalled();
		expect(mocks.del).not.toHaveBeenCalled();
	});
});

describe("presence", () => {
	test("is the hub's report; signed out there is none", async () => {
		const reports = {
			acct_a: { state: "online", last_seen_at: "2026-11-03T10:00:00Z" },
		};
		mocks.get.mockResolvedValue(reports);
		await expect(new BotState(backend()).getPresence()).resolves.toEqual(
			reports,
		);
		expect(mocks.get).toHaveBeenCalledWith(profile, "bots/presence", auth);

		mocks.get.mockClear();
		await expect(new BotState(backend(false)).getPresence()).resolves.toEqual(
			{},
		);
		expect(mocks.get).not.toHaveBeenCalled();
	});
});

describe("the bot host's turns", () => {
	const turn = (bot_id: string) => ({
		id: "bt_1",
		bot_id,
		conversation_id: "c1",
		status: "running" as const,
	});

	test("lists them and hears every change", async () => {
		const state = await signedInState();
		mocks.invoke.mockResolvedValue([turn("local_a")]);
		await expect(state.turns.list()).resolves.toEqual([turn("local_a")]);
		expect(mocks.invoke).toHaveBeenCalledWith("bot_host_turns");

		const unlisten = vi.fn();
		let deliver: (event: { payload: unknown }) => void = () => {};
		mocks.listen.mockImplementation(async (_name, handler) => {
			deliver = handler;
			return unlisten;
		});
		const seen: unknown[] = [];
		const stop = await state.turns.subscribe((next) => seen.push(next));
		expect(mocks.listen).toHaveBeenCalledWith(
			"bot-host://turn",
			expect.any(Function),
		);
		deliver({ payload: turn("local_a") });
		expect(seen).toEqual([turn("local_a")]);
		stop();
		expect(unlisten).toHaveBeenCalled();
	});

	test("cancels a local bot's turn on this computer", async () => {
		const state = await signedInState();
		mocks.invoke.mockResolvedValue(true);

		await state.turns.cancel(turn("local_a"));

		expect(mocks.invoke).toHaveBeenCalledWith("bot_host_cancel_turn", {
			turnId: "bt_1",
		});
		expect(mocks.post).not.toHaveBeenCalled();
	});

	test("cancels an account bot's turn at the hub", async () => {
		const state = await signedInState();
		mocks.post.mockResolvedValue({});

		await state.turns.cancel(turn("acct_a"));

		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"bots/acct_a/turns/bt_1/cancel",
			undefined,
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"bot_host_cancel_turn",
			expect.anything(),
		);
	});
});
