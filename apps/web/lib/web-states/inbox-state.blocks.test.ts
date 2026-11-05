import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
	apiPost: vi.fn(),
	apiPatch: vi.fn(),
	apiPut: vi.fn(),
	apiDelete: vi.fn(),
}));
vi.mock("./api-utils", () => mocks);

import { WebInboxState } from "./inbox-state";

const auth = { user: { access_token: "token" } };
const inbox = () => new WebInboxState({ auth } as never);

describe("blocked people and bots", () => {
	beforeEach(() => vi.resetAllMocks());

	test("lists the account's blocks", async () => {
		const blocks = [
			{
				principal_id: "bot/1",
				kind: "bot",
				created_at: "2026-11-05T00:00:00Z",
			},
		];
		mocks.apiGet.mockResolvedValue({ blocks });
		expect(await inbox().listBlocks()).toEqual(blocks);
		expect(mocks.apiGet).toHaveBeenCalledWith("inbox/blocks", auth);
	});

	test("encodes principal ids for block and unblock", async () => {
		await inbox().blockPrincipal("user/a:b");
		await inbox().unblockPrincipal("user/a:b");
		expect(mocks.apiPut).toHaveBeenCalledWith(
			"inbox/blocks/user%2Fa%3Ab",
			{},
			auth,
		);
		expect(mocks.apiDelete).toHaveBeenCalledWith(
			"inbox/blocks/user%2Fa%3Ab",
			auth,
		);
	});

	test("does not report a refused block as successful", async () => {
		mocks.apiPut.mockRejectedValue(new Error("You cannot block your own bot"));
		await expect(inbox().blockPrincipal("mine")).rejects.toThrow(
			"You cannot block your own bot",
		);
	});
});
