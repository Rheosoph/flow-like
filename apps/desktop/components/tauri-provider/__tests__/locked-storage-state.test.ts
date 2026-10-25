import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ fetcher: vi.fn() }));
vi.mock("../../../lib/api", () => ({ fetcher: mocks.fetcher }));

import {
	LOCKED_STORAGE_OFFLINE_ERROR,
	LockedStorageState,
} from "../locked-storage-state";

function backend(offline: boolean) {
	return {
		isOffline: vi.fn().mockResolvedValue(offline),
		profile: { id: "profile" },
		auth: { user: { access_token: "token" } },
	};
}

describe("desktop locked storage", () => {
	beforeEach(() => mocks.fetcher.mockReset());

	test("offline apps report locked storage as disabled and refuse calls", async () => {
		const state = new LockedStorageState(backend(true) as never);
		await expect(state.getLockedStorageStatus("app")).resolves.toMatchObject({
			enabled: false,
		});
		await expect(
			state.listLockedItems("app", { folder: "Übersicht" }),
		).rejects.toThrow(LOCKED_STORAGE_OFFLINE_ERROR);
		await expect(state.downloadLockedItem("app", "item")).rejects.toThrow(
			LOCKED_STORAGE_OFFLINE_ERROR,
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("online apps call the hub's locked routes with JSON bodies", async () => {
		const online = backend(false);
		mocks.fetcher.mockResolvedValue({ enabled: true });
		const state = new LockedStorageState(online as never);
		await state.getLockedStorageStatus("app");
		await state.listLockedItems("app", { scope: "user", folder: "Übersicht" });
		expect(mocks.fetcher).toHaveBeenNthCalledWith(
			1,
			online.profile,
			"apps/app/locked/status",
			{ method: "GET", body: undefined, signal: undefined },
			online.auth,
		);
		expect(mocks.fetcher).toHaveBeenNthCalledWith(
			2,
			online.profile,
			"apps/app/locked/list",
			{
				method: "POST",
				body: JSON.stringify({ scope: "user", folder: "Übersicht" }),
				signal: undefined,
			},
			online.auth,
		);
	});
});
