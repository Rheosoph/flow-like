import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock("./api-utils", () => ({ apiFetch: mocks.apiFetch }));

import { WebLockedStorageState } from "./locked-storage-state";

describe("web locked storage", () => {
	beforeEach(() => mocks.apiFetch.mockReset());

	test("calls the app's locked routes with the session auth", async () => {
		const auth = { user: { access_token: "token" } };
		const state = new WebLockedStorageState({ auth } as never);
		mocks.apiFetch.mockResolvedValue({});
		await state.getLockedStorageStatus("app");
		await state.purgeLockedItem("app", "item", "version");
		expect(mocks.apiFetch).toHaveBeenNthCalledWith(
			1,
			"apps/app/locked/status",
			{ method: "GET", body: undefined, signal: undefined },
			auth,
		);
		expect(mocks.apiFetch).toHaveBeenNthCalledWith(
			2,
			"apps/app/locked/purge",
			{
				method: "POST",
				body: JSON.stringify({ item_id: "item", version_id: "version" }),
				signal: undefined,
			},
			auth,
		);
	});
});
