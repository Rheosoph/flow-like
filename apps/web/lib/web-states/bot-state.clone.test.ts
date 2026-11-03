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

describe("duplicating a bot on the web", () => {
	beforeEach(() => vi.resetAllMocks());

	test("asks the hub for a copy", async () => {
		const copy = { id: "bot_copy", name: "Scout copy" };
		mocks.apiPost.mockResolvedValue(copy);
		await expect(
			new WebBotState({ auth } as never).cloneBot("bot 1"),
		).resolves.toEqual(copy);
		expect(mocks.apiPost).toHaveBeenCalledWith(
			"bots/bot%201/clone",
			undefined,
			auth,
		);
	});

	test("an answer without a bot is an error", async () => {
		mocks.apiPost.mockResolvedValue(undefined);
		await expect(
			new WebBotState({ auth } as never).cloneBot("bot_1"),
		).rejects.toThrow(
			"Unexpected response while trying to duplicate bot bot_1",
		);
	});
});
