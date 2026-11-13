import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
	apiPost: vi.fn(),
	apiPatch: vi.fn(),
	apiPut: vi.fn(),
	apiDelete: vi.fn(),
}));
vi.mock("./api-utils", () => mocks);

import { WebBotState } from "./bot-state";

const auth = { user: { access_token: "token" } };
const grant = {
	id: "a>b",
	from_bot_id: "bot_a",
	to_bot_id: "bot b",
	granted_at: "2026-11-03T10:00:00Z",
	granted_by: "me",
};

describe("what a bot may ask on the web", () => {
	beforeEach(() => vi.resetAllMocks());

	test("lists both directions from the hub", async () => {
		mocks.apiGet.mockResolvedValue({ granted: [grant], received: [] });
		await expect(
			new WebBotState({ auth } as never).listPermissions("bot_a"),
		).resolves.toEqual({ granted: [grant], received: [] });
		expect(mocks.apiGet).toHaveBeenCalledWith("bots/bot_a/permissions", auth);
	});

	test("a missing list reads as no grants", async () => {
		mocks.apiGet.mockResolvedValue({});
		await expect(
			new WebBotState({ auth } as never).listPermissions("bot_a"),
		).resolves.toEqual({ granted: [], received: [] });
	});

	test("grants and revokes by the asking bot and the asked bot", async () => {
		mocks.apiPut.mockResolvedValue(grant);
		const state = new WebBotState({ auth } as never);
		await expect(state.grantPermission("bot_a", "bot b")).resolves.toEqual(
			grant,
		);
		await state.revokePermission("bot_a", "bot b");
		expect(mocks.apiPut).toHaveBeenCalledWith(
			"bots/bot_a/permissions/bot%20b",
			undefined,
			auth,
		);
		expect(mocks.apiDelete).toHaveBeenCalledWith(
			"bots/bot_a/permissions/bot%20b",
			auth,
		);
	});

	test("presence is the hub's report", async () => {
		const reports = { bot_a: { state: "offline", last_seen_at: null } };
		mocks.apiGet.mockResolvedValue(reports);
		await expect(
			new WebBotState({ auth } as never).getPresence(),
		).resolves.toEqual(reports);
		expect(mocks.apiGet).toHaveBeenCalledWith("bots/presence", auth);
	});
});

describe("what a bot may do without asking on the web", () => {
	beforeEach(() => vi.resetAllMocks());
	const toolGrant = {
		tool: "shell_run",
		scope: "command:git",
		granted_at: "2026-11-03T10:00:00Z",
		granted_by: "me",
	};

	test("lists, allows and takes back a grant by tool and scope", async () => {
		mocks.apiGet.mockResolvedValue({ grants: [toolGrant] });
		mocks.apiPut.mockResolvedValue(toolGrant);
		const state = new WebBotState({ auth } as never);

		await expect(state.listToolGrants("bot_a")).resolves.toEqual([toolGrant]);
		await expect(
			state.putToolGrant("bot_a", "shell_run", "command:git"),
		).resolves.toEqual(toolGrant);
		await state.deleteToolGrant("bot_a", "shell_run", "command:git");

		expect(mocks.apiGet).toHaveBeenCalledWith("bots/bot_a/tool-grants", auth);
		expect(mocks.apiPut).toHaveBeenCalledWith(
			"bots/bot_a/tool-grants",
			{ tool: "shell_run", scope: "command:git" },
			auth,
		);
		expect(mocks.apiDelete).toHaveBeenCalledWith(
			"bots/bot_a/tool-grants",
			auth,
			{
				tool: "shell_run",
				scope: "command:git",
			},
		);
	});

	test("a missing list reads as no grants", async () => {
		mocks.apiGet.mockResolvedValue({});
		await expect(
			new WebBotState({ auth } as never).listToolGrants("bot_a"),
		).resolves.toEqual([]);
	});

	test("answers an approval on the parked turn", async () => {
		mocks.apiPost.mockResolvedValue({});
		await new WebBotState({ auth } as never).answerApproval(
			"bot_a",
			"turn 1",
			"ap-1",
			"once",
		);
		expect(mocks.apiPost).toHaveBeenCalledWith(
			"bots/bot_a/turns/turn%201/approval",
			{ approval_id: "ap-1", decision: "once" },
			auth,
		);
	});

	test("a batch answer lists the calls whose answer differs", async () => {
		mocks.apiPost.mockResolvedValue({});
		await new WebBotState({ auth } as never).answerApproval(
			"bot_a",
			"turn_1",
			"ap-b",
			"once",
			[{ args_hash: "h2", decision: "deny" }],
		);
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
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
