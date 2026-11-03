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

describe("a bot's workspace on the web", () => {
	beforeEach(() => vi.resetAllMocks());

	test("asks the hub for it, which makes it on first use", async () => {
		mocks.apiPost.mockResolvedValue({ app_id: "app-ws" });
		await expect(
			new WebBotState({ auth } as never).getWorkspace("bot 1"),
		).resolves.toEqual({ app_id: "app-ws" });
		expect(mocks.apiPost).toHaveBeenCalledWith(
			"bots/bot%201/workspace",
			undefined,
			auth,
		);
	});

	test("an answer without an app is an error, not an empty workspace", async () => {
		mocks.apiPost.mockResolvedValue({ app_id: "" });
		await expect(
			new WebBotState({ auth } as never).getWorkspace("bot_1"),
		).rejects.toThrow(
			"Unexpected response while opening the workspace of bot bot_1",
		);
	});
});
