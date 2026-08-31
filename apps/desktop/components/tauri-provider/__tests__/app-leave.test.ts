import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	put: vi.fn(),
	discardOfflineSyncForApp: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));

vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	put: mocks.put,
}));

// The barrel drags the whole component library into the module graph; this suite
// only needs the enums and the helpers `app-state` reads at runtime.
vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await vi.importActual<Record<string, unknown>>(
		"@flow-like/flow-like-ui/lib/schema/app/app",
	)),
	IExecutionStage: { Dev: "Dev" },
	ILogLevel: { Debug: "Debug" },
	injectDataFunction: vi.fn(),
	discardOfflineSyncForApp: mocks.discardOfflineSyncForApp,
}));

vi.mock("../../../lib/apps-db", () => ({
	appsDB: {
		visibility: { get: vi.fn(), put: vi.fn() },
	},
}));

import { ApiResponseError } from "../../../lib/api-error";
import type { TauriBackend } from "../../tauri-provider";
import { AppState } from "../app-state";

const APP = "app-1";
const SUB = "user-7";

const apiError = (status: number) =>
	new ApiResponseError({ status, message: `HTTP ${status}` });

function memberBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(false),
		isLocalOnly: vi.fn().mockResolvedValue(false),
		profile: { hub: "hub.example" },
		auth: {
			isAuthenticated: true,
			user: { access_token: "token", profile: { sub: SUB } },
		},
		queryClient: {},
	} as unknown as TauriBackend;
}

describe("AppState.leaveApp", () => {
	beforeEach(() => {
		mocks.invoke.mockReset().mockResolvedValue(undefined);
		mocks.fetcher.mockReset();
		mocks.discardOfflineSyncForApp.mockReset().mockResolvedValue(0);
	});

	test("removes only the caller's own membership, then the local copy", async () => {
		mocks.fetcher.mockResolvedValue(undefined);

		await new AppState(memberBackend()).leaveApp(APP);

		expect(mocks.fetcher).toHaveBeenCalledWith(
			{ hub: "hub.example" },
			`apps/${APP}/team/${SUB}`,
			{ method: "DELETE" },
			expect.anything(),
		);
		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
	});

	test("clears queued offline edits so the outbox cannot retry forever", async () => {
		mocks.fetcher.mockResolvedValue(undefined);

		await new AppState(memberBackend()).leaveApp(APP);

		expect(mocks.discardOfflineSyncForApp).toHaveBeenCalledWith(
			APP,
			"app-left",
		);
	});

	test("still clears the local copy when the membership is already gone", async () => {
		mocks.fetcher.mockRejectedValue(apiError(404));

		await new AppState(memberBackend()).leaveApp(APP);

		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
	});

	test("keeps the local copy when the server refuses — an owner cannot leave", async () => {
		mocks.fetcher.mockRejectedValue(apiError(403));

		await expect(new AppState(memberBackend()).leaveApp(APP)).rejects.toThrow(
			ApiResponseError,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("refuses a local-only app, which has no team to leave", async () => {
		const backend = memberBackend();
		(backend.isLocalOnly as ReturnType<typeof vi.fn>).mockResolvedValue(true);

		await expect(new AppState(backend).leaveApp(APP)).rejects.toThrow(
			/local-only/,
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("refuses when no user is signed in", async () => {
		const backend = memberBackend();
		(backend as unknown as { auth: { user?: unknown } }).auth.user = undefined;

		await expect(new AppState(backend).leaveApp(APP)).rejects.toThrow(
			/No signed-in user/,
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});
});
