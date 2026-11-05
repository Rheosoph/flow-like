import { beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	post: vi.fn(),
	visibilityPut: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../../lib/api", () => ({
	get: vi.fn(),
	post: mocks.post,
	put: vi.fn(),
	patch: vi.fn(),
	del: vi.fn(),
}));
vi.mock("../../../lib/apps-db", () => ({
	appsDB: { visibility: { put: mocks.visibilityPut } },
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

beforeEach(() => {
	for (const mock of Object.values(mocks)) mock.mockReset();
	mocks.visibilityPut.mockResolvedValue(undefined);
});

describe("a bot's workspace on the desktop", () => {
	test("a bot on this computer gets its workspace from the local store", async () => {
		localStore({ local_bots_workspace: () => ({ app_id: "app-local" }) });

		await expect(
			new BotState(signedOut()).getWorkspace("bot_local"),
		).resolves.toEqual({ app_id: "app-local" });

		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_workspace", {
			botId: "bot_local",
		});
		expect(mocks.post).not.toHaveBeenCalled();
		expect(mocks.visibilityPut).not.toHaveBeenCalled();
	});

	test("says why when this computer's store can't make one", async () => {
		localStore({});

		await expect(
			new BotState(signedOut()).getWorkspace("bot_local"),
		).rejects.toThrow(
			"Could not open the workspace of local bot bot_local: Command local_bots_workspace not found",
		);
	});

	test("a refusal keeps the store's code and words", async () => {
		localStore({
			local_bots_workspace: () => {
				throw {
					code: "BOT_WORKSPACE_REFUSED",
					message: "Flux can't have a workspace yet",
				};
			},
		});

		await expect(
			new BotState(signedOut()).getWorkspace("bot_flux"),
		).rejects.toMatchObject({
			name: "LocalStoreError",
			code: "BOT_WORKSPACE_REFUSED",
			message:
				"Could not open the workspace of local bot bot_flux: Flux can't have a workspace yet",
		});
	});

	test("a local answer without an app is an error", async () => {
		localStore({ local_bots_workspace: () => ({ app_id: "" }) });

		await expect(
			new BotState(signedOut()).getWorkspace("bot_local"),
		).rejects.toThrow(
			"Could not open the workspace of local bot bot_local: this computer returned no app",
		);
	});

	test("an account bot's workspace comes from the hub and is registered as Private first", async () => {
		localStore({ local_bots_get: () => null, local_bots_list: () => [] });
		mocks.post.mockResolvedValue({ app_id: "app-hub" });

		await expect(
			new BotState(signedIn()).getWorkspace("bot_remote"),
		).resolves.toEqual({ app_id: "app-hub" });

		expect(mocks.post).toHaveBeenCalledWith(
			profile,
			"bots/bot_remote/workspace",
			undefined,
			auth,
		);
		expect(mocks.visibilityPut).toHaveBeenCalledWith({
			appId: "app-hub",
			visibility: "Private",
		});
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"local_bots_workspace",
			expect.anything(),
		);
	});

	test("an answer without an app registers nothing", async () => {
		localStore({ local_bots_get: () => null });
		mocks.post.mockResolvedValue({});

		await expect(
			new BotState(signedIn()).getWorkspace("bot_remote"),
		).rejects.toThrow(
			"Unexpected response while opening the workspace of bot bot_remote",
		);
		expect(mocks.visibilityPut).not.toHaveBeenCalled();
	});
});
