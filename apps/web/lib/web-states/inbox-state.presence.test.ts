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

describe("conversation presence", () => {
	beforeEach(() => vi.resetAllMocks());

	test("lists the unfinished turns", async () => {
		mocks.apiGet.mockResolvedValue({ turns: [{ id: "bt_1", bot_id: "b" }] });
		const turns = await inbox().listConversationTurns("conv 1");
		expect(mocks.apiGet).toHaveBeenCalledWith(
			"inbox/conversations/conv%201/turns",
			auth,
		);
		expect(turns).toEqual([{ id: "bt_1", bot_id: "b" }]);
	});

	test("an empty response has no turns", async () => {
		mocks.apiGet.mockResolvedValue({});
		expect(await inbox().listConversationTurns("c1")).toEqual([]);
	});

	test("asks for realtime access with a POST", async () => {
		mocks.apiPost.mockResolvedValue({
			jwt: "j",
			encryption_key: "k",
			key_id: "i",
		});
		const access = await inbox().getConversationRealtimeAccess("c1");
		expect(mocks.apiPost).toHaveBeenCalledWith(
			"inbox/conversations/c1/realtime",
			undefined,
			auth,
		);
		expect(access.key_id).toBe("i");
	});
});
