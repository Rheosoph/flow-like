import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiDelete: vi.fn() }));
vi.mock("./api-utils", () => mocks);

import { WebInboxState } from "./inbox-state";

const auth = { user: { access_token: "token" } };

describe("deleting a conversation from the account", () => {
	beforeEach(() => vi.resetAllMocks());

	test("deletes only the selected conversation using the signed-in account", async () => {
		const inbox = new WebInboxState({ auth } as never);
		await inbox.deleteConversation("room/a:b");
		expect(mocks.apiDelete).toHaveBeenCalledWith(
			"inbox/conversations/room%2Fa%3Ab",
			auth,
		);
	});

	test("passes a refusal back to the confirmation dialog", async () => {
		mocks.apiDelete.mockRejectedValue(new Error("Conversation unavailable"));
		const inbox = new WebInboxState({ auth } as never);
		await expect(inbox.deleteConversation("room")).rejects.toThrow(
			"Conversation unavailable",
		);
	});
});
