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

const apiError = (status: number) =>
	new ApiResponseError({ status, message: `HTTP ${status}` });

function onlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(false),
		profile: { hub: "hub.example" },
		auth: { isAuthenticated: true, user: { access_token: "token" } },
		queryClient: {},
	} as unknown as TauriBackend;
}

describe("AppState.deleteApp", () => {
	beforeEach(() => {
		mocks.invoke.mockReset().mockResolvedValue(undefined);
		mocks.fetcher.mockReset();
		mocks.discardOfflineSyncForApp.mockReset().mockResolvedValue(0);
	});

	test("removes the local copy after the server accepted the delete", async () => {
		mocks.fetcher.mockResolvedValue(undefined);

		await new AppState(onlineBackend()).deleteApp(APP);

		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
	});

	test("removes the local copy when the app is already gone on the server", async () => {
		mocks.fetcher.mockRejectedValue(apiError(404));

		await new AppState(onlineBackend()).deleteApp(APP);

		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
	});

	test("removes the local copy when the membership cascaded away with the app", async () => {
		mocks.fetcher
			.mockRejectedValueOnce(apiError(403))
			.mockRejectedValueOnce(apiError(404));

		await new AppState(onlineBackend()).deleteApp(APP);

		expect(mocks.fetcher).toHaveBeenCalledTimes(2);
		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
	});

	test("keeps the local copy when the app is still readable but not ours to delete", async () => {
		mocks.fetcher
			.mockRejectedValueOnce(apiError(403))
			.mockResolvedValueOnce({ id: APP });

		await expect(new AppState(onlineBackend()).deleteApp(APP)).rejects.toThrow(
			ApiResponseError,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("keeps the local copy when the server failed for another reason", async () => {
		mocks.fetcher.mockRejectedValue(apiError(500));

		await expect(new AppState(onlineBackend()).deleteApp(APP)).rejects.toThrow(
			ApiResponseError,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("deletes offline apps without contacting the server", async () => {
		const backend = onlineBackend();
		(backend.isOffline as ReturnType<typeof vi.fn>).mockResolvedValue(true);

		await new AppState(backend).deleteApp(APP);

		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
	});

	test("clears queued offline edits alongside the local copy", async () => {
		mocks.fetcher.mockResolvedValue(undefined);

		await new AppState(onlineBackend()).deleteApp(APP);

		expect(mocks.discardOfflineSyncForApp).toHaveBeenCalledWith(
			APP,
			"app-deleted",
		);
	});
});
