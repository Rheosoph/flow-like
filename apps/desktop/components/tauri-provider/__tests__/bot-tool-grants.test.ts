import type { IBot } from "@flow-like/flow-like-ui";
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

import { BotState } from "../bot-state";

const profile = { id: "profile-1", hub: "hub.example" };
const auth = {
	isAuthenticated: true,
	isLoading: false,
	user: { access_token: "token", profile: { sub: "user-1" } },
};
const grant = {
	tool: "shell_run",
	scope: "command:git",
	granted_at: "2026-11-03T10:00:00Z",
	granted_by: "me",
};

function bot(id: string, local = false): IBot {
	return {
		id,
		owner_id: local ? "local" : "user-1",
		name: id,
		location: "desktop",
		...(local ? { local: true } : {}),
	} as unknown as IBot;
}

function localStore(handlers: Record<string, (args: never) => unknown>) {
	mocks.invoke.mockImplementation(async (command: string, args?: never) => {
		const handler = handlers[command];
		if (!handler) throw `Command ${command} not found`;
		return handler(args as never);
	});
}

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
});
afterEach(() => vi.restoreAllMocks());

describe("tool grants and approvals of a local bot", () => {
	const signedOut = () => new BotState({ profile } as unknown as TauriBackend);

	test("go through the local store's commands", async () => {
		localStore({
			local_bot_tool_grants: () => [grant],
			local_bot_put_tool_grant: () => grant,
			local_bot_delete_tool_grant: () => null,
			local_bot_answer_approval: () => null,
		});
		const state = signedOut();

		expect(await state.listToolGrants("bot_l")).toEqual([grant]);
		expect(
			await state.putToolGrant("bot_l", "shell_run", "command:git"),
		).toEqual(grant);
		await state.deleteToolGrant("bot_l", "shell_run", "command:git");
		await state.answerApproval("bot_l", "turn_1", "ap-1", "always");

		expect(mocks.invoke).toHaveBeenCalledWith("local_bot_tool_grants", {
			botId: "bot_l",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_bot_put_tool_grant", {
			botId: "bot_l",
			tool: "shell_run",
			scope: "command:git",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_bot_delete_tool_grant", {
			botId: "bot_l",
			tool: "shell_run",
			scope: "command:git",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("local_bot_answer_approval", {
			botId: "bot_l",
			turnId: "turn_1",
			approvalId: "ap-1",
			decision: "always",
		});
		for (const call of [mocks.get, mocks.post, mocks.put, mocks.del]) {
			expect(call).not.toHaveBeenCalled();
		}
	});

	test("a refused write names the operation", async () => {
		localStore({
			local_bot_put_tool_grant: () => {
				throw { code: "INVALID", message: "Scope has a comma" };
			},
		});
		await expect(signedOut().putToolGrant("bot_l", "t", "a,b")).rejects.toThrow(
			"Could not allow t (a,b) for local bot bot_l: Scope has a comma",
		);
	});
});

describe("tool grants and approvals of an account bot", () => {
	async function signedIn() {
		mocks.get.mockResolvedValueOnce({ bots: [bot("bot_a")] });
		localStore({ local_bots_list: () => [] });
		const state = new BotState({ profile, auth } as unknown as TauriBackend);
		await state.listBots();
		mocks.get.mockReset();
		mocks.invoke.mockReset();
		return state;
	}

	test("go through the hub's endpoints", async () => {
		const state = await signedIn();
		mocks.get.mockResolvedValue({ grants: [grant] });
		mocks.put.mockResolvedValue(grant);

		expect(await state.listToolGrants("bot_a")).toEqual([grant]);
		await state.putToolGrant("bot_a", "shell_run", "command:git");
		await state.deleteToolGrant("bot_a", "shell_run", "command:git");
		await state.answerApproval("bot_a", "turn 1", "ap-1", "deny");

		expect(mocks.get).toHaveBeenCalledWith(
			profile,
			"bots/bot_a/tool-grants",
			auth,
		);
		expect(mocks.put).toHaveBeenCalledWith(
			profile,
			"bots/bot_a/tool-grants",
			{ tool: "shell_run", scope: "command:git" },
			auth,
		);
		expect(mocks.del).toHaveBeenCalledWith(
			profile,
			"bots/bot_a/tool-grants",
			{ tool: "shell_run", scope: "command:git" },
			auth,
		);
		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"bots/bot_a/turns/turn%201/approval",
			{ approval_id: "ap-1", decision: "deny" },
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("a batch answer carries the calls whose answer differs", async () => {
		const state = await signedIn();
		await state.answerApproval("bot_a", "turn_1", "ap-b", "once", [
			{ args_hash: "h2", decision: "deny" },
		]);
		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"bots/bot_a/turns/turn_1/approval",
			{
				approval_id: "ap-b",
				decision: "once",
				items: [{ args_hash: "h2", decision: "deny" }],
			},
			auth,
		);
	});
});

describe("a batch answer of a local bot", () => {
	test("hands the differing calls to the local store", async () => {
		localStore({ local_bot_answer_approval: () => null });
		await new BotState({ profile } as unknown as TauriBackend).answerApproval(
			"bot_l",
			"turn_1",
			"ap-b",
			"always",
			[{ args_hash: "h2", decision: "once" }],
		);
		expect(mocks.invoke).toHaveBeenCalledWith("local_bot_answer_approval", {
			botId: "bot_l",
			turnId: "turn_1",
			approvalId: "ap-b",
			decision: "always",
			items: [{ args_hash: "h2", decision: "once" }],
		});
	});
});
