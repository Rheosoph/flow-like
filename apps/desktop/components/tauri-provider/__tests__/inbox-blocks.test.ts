import { beforeEach, describe, expect, test, vi } from "vitest";
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
vi.mock("../../../lib/api", () => mocks);

import { InboxState } from "../inbox-state";

const profile = { id: "profile-1", hub: "hub.example" };
const auth = {
	isAuthenticated: true,
	isLoading: false,
	user: { access_token: "token", profile: { sub: "user-1" } },
};
const inbox = (signedIn = true) =>
	new InboxState({
		profile,
		auth: signedIn ? auth : undefined,
	} as unknown as TauriBackend);

describe("account blocks on desktop", () => {
	beforeEach(() => vi.resetAllMocks());

	test("uses the signed-in hub and escapes the principal id", async () => {
		const blocks = [
			{
				principal_id: "bot/1",
				kind: "bot",
				created_at: "2026-11-05T00:00:00Z",
			},
		];
		mocks.get.mockResolvedValue({ blocks });
		const state = inbox();
		expect(await state.listBlocks()).toEqual(blocks);
		await state.blockPrincipal("bot/1");
		await state.unblockPrincipal("bot/1");
		expect(mocks.get).toHaveBeenCalledWith(profile, "inbox/blocks", auth);
		expect(mocks.put).toHaveBeenCalledWith(
			profile,
			"inbox/blocks/bot%2F1",
			{},
			auth,
		);
		expect(mocks.del).toHaveBeenCalledWith(
			profile,
			"inbox/blocks/bot%2F1",
			undefined,
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("signed-out accounts cannot silently block or unblock", async () => {
		const state = inbox(false);
		expect(await state.listBlocks()).toEqual([]);
		await expect(state.blockPrincipal("user-2")).rejects.toThrow("Sign in");
		await expect(state.unblockPrincipal("user-2")).rejects.toThrow("Sign in");
		for (const method of Object.values(mocks)) {
			expect(method).not.toHaveBeenCalled();
		}
	});
});
