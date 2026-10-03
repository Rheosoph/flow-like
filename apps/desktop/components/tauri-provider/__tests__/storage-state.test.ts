import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ fetcher: vi.fn(), invoke: vi.fn() }));
vi.mock("@flow-like/flow-like-ui", () => ({
	BulkUploadAbortError: class extends Error {},
	assertBulkUploadSucceeded: vi.fn(),
	requestPrefixesInBatches: vi.fn(),
	runBulkUpload: vi.fn(),
	storageDisplayName: vi.fn(),
	toUploadTasks: vi.fn(),
	uploadToSignedUrl: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui/lib/stable-asset-url", () => ({
	stabilizeSignedUrls: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/path", () => ({ dirname: vi.fn(), resolve: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({
	mkdir: vi.fn(),
	open: vi.fn(),
	remove: vi.fn(),
}));
vi.mock("../../../lib/api", () => ({ fetcher: mocks.fetcher }));

import { StorageState } from "../storage-state";

const routes = [
	["listStorageItems", "apps/app/data/list", "storage_list"],
	["listStorageItemsUser", "apps/app/data/user/list", "storage_user_list"],
] as const;

describe("desktop storage listing refresh", () => {
	beforeEach(() => {
		vi.resetAllMocks();
		mocks.fetcher.mockResolvedValue([]);
		mocks.invoke.mockResolvedValue([]);
	});

	test.each(routes)(
		"%s refreshes remote storage through the URL",
		async (method, endpoint) => {
			const backend = {
				isOffline: vi.fn().mockResolvedValue(false),
				profile: {},
				auth: {},
				queryClient: {},
			};
			const state = new StorageState(backend as never);
			const request = {
				method: "POST",
				body: JSON.stringify({ prefix: "docs" }),
			};
			for (const options of [undefined, { refresh: false }]) {
				await state[method]("app", "docs", options);
				expect(mocks.fetcher).toHaveBeenLastCalledWith(
					backend.profile,
					endpoint,
					request,
					backend.auth,
				);
			}
			await state[method]("app", "docs", { refresh: true });
			expect(mocks.fetcher).toHaveBeenLastCalledWith(
				backend.profile,
				`${endpoint}?refresh=true`,
				request,
				backend.auth,
			);
			expect(mocks.invoke).not.toHaveBeenCalled();
			mocks.fetcher.mockRejectedValueOnce(new Error("denied"));
			await expect(
				state[method]("app", "docs", { refresh: true }),
			).rejects.toThrow("denied");
		},
	);

	test.each(routes)(
		"%s keeps offline listings local",
		async (method, _endpoint, command) => {
			const state = new StorageState({
				isOffline: vi.fn().mockResolvedValue(true),
			} as never);
			await state[method]("app", "docs", { refresh: true });
			expect(mocks.invoke).toHaveBeenCalledWith(command, {
				appId: "app",
				prefix: "docs",
			});
			expect(mocks.fetcher).not.toHaveBeenCalled();
		},
	);
});
