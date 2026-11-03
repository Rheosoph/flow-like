import { beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	post: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../../lib/api", () => ({
	get: vi.fn(),
	post: mocks.post,
	put: vi.fn(),
	patch: vi.fn(),
	del: vi.fn(),
}));

import { BotState } from "../bot-state";

const profile = { id: "profile-1", hub: "hub.example" };
const auth = {
	isAuthenticated: true,
	isLoading: false,
	user: { access_token: "token", profile: { sub: "user-1" } },
};

function signedIn(): TauriBackend {
	return { profile, auth } as unknown as TauriBackend;
}

function signedOut(): TauriBackend {
	return { profile, auth: undefined } as unknown as TauriBackend;
}

/** This computer's store: the commands it knows, everything else rejects as Tauri does. */
function localStore(handlers: Record<string, (args: unknown) => unknown>) {
	mocks.invoke.mockImplementation(async (command: string, args?: unknown) => {
		const handler = handlers[command];
		if (!handler) throw `Command ${command} not found`;
		return handler(args);
	});
}

const copy = { id: "bot_copy", name: "Scout copy", system: null };

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
});

describe("duplicating a bot on the desktop", () => {
	test("a bot on this computer is copied by the local store and the copy stays here", async () => {
		localStore({ local_bots_clone: () => copy });

		await expect(
			new BotState(signedOut()).cloneBot("bot_local"),
		).resolves.toEqual({ ...copy, local: true });

		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_clone", {
			botId: "bot_local",
		});
		expect(mocks.post).not.toHaveBeenCalled();
	});

	test("says why when this computer's store can't copy it", async () => {
		localStore({});

		await expect(
			new BotState(signedOut()).cloneBot("bot_local"),
		).rejects.toThrow(
			"Could not duplicate local bot bot_local: Command local_bots_clone not found",
		);
	});

	test("an account bot is copied by the hub", async () => {
		localStore({ local_bots_get: () => null, local_bots_list: () => [] });
		mocks.post.mockResolvedValue(copy);

		await expect(
			new BotState(signedIn()).cloneBot("bot_remote"),
		).resolves.toEqual(copy);

		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"bots/bot_remote/clone",
			undefined,
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"local_bots_clone",
			expect.anything(),
		);
	});

	test("an answer without a bot is an error", async () => {
		localStore({ local_bots_get: () => null });
		mocks.post.mockResolvedValue(null);

		await expect(
			new BotState(signedIn()).cloneBot("bot_remote"),
		).rejects.toThrow(
			"Unexpected response while trying to duplicate bot bot_remote",
		);
	});
});
