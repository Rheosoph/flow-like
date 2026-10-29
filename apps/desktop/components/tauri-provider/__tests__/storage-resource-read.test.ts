import { beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), fetcher: vi.fn() }));
vi.mock("@flow-like/flow-like-ui", () => ({
	BulkUploadAbortError: class extends Error {},
	assertBulkUploadSucceeded: vi.fn(),
	requestPrefixesInBatches: vi.fn(),
	runBulkUpload: vi.fn(),
	storageDisplayName: vi.fn(),
	toUploadTasks: vi.fn(),
	uploadToSignedUrl: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/path", () => ({ dirname: vi.fn(), resolve: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({
	mkdir: vi.fn(),
	open: vi.fn(),
	remove: vi.fn(),
	rename: vi.fn(),
}));
vi.mock("../../../lib/api", () => ({ fetcher: mocks.fetcher }));

import { StorageState } from "../storage-state";

beforeEach(() => vi.resetAllMocks());

test("offline resource reads use native bytes for app and user scopes", async () => {
	const bytes = new Uint8Array([0, 255, 128]).buffer;
	mocks.invoke.mockResolvedValue(bytes);
	const state = new StorageState({ isOffline: async () => true } as never);
	for (const [prefix, userScoped] of [
		["apps/a/upload/image.png", false],
		["users/u/apps/a/image.png", true],
	] as const) {
		expect(await state.readStorageItem("a", prefix, userScoped)).toBe(bytes);
		expect(mocks.invoke).toHaveBeenLastCalledWith("storage_resource_read", {
			appId: "a",
			prefix,
			userScoped,
		});
	}
	expect(mocks.fetcher).not.toHaveBeenCalled();
});

test("online resource reads explicitly defer to authorized remote URL resolution", async () => {
	const state = new StorageState({ isOffline: async () => false } as never);
	expect(
		await state.readStorageItem("a", "apps/a/upload/image.png", false),
	).toBeNull();
	expect(mocks.invoke).not.toHaveBeenCalled();
	expect(mocks.fetcher).not.toHaveBeenCalled();
});

test("native denial or unavailable command propagates instead of producing an asset URL", async () => {
	const state = new StorageState({ isOffline: async () => true } as never);
	for (const reason of ["permission denied", "command not found"]) {
		mocks.invoke.mockRejectedValueOnce(new Error(reason));
		await expect(
			state.readStorageItem("a", "apps/a/upload/image.png", false),
		).rejects.toThrow(reason);
	}
	expect(
		mocks.invoke.mock.calls.every(
			([command]) => command === "storage_resource_read",
		),
	).toBe(true);
	expect(mocks.fetcher).not.toHaveBeenCalled();
});
