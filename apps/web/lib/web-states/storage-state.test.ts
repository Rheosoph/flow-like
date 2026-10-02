import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiPost: vi.fn() }));
vi.mock("@flow-like/flow-like-ui", () => ({
	assertBulkUploadSucceeded: vi.fn(),
	requestPrefixesInBatches: vi.fn(),
	runBulkUpload: vi.fn(),
	toUploadTasks: vi.fn(),
	uploadToSignedUrl: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui/lib/stable-asset-url", () => ({
	stabilizeSignedUrls: vi.fn(),
}));
vi.mock("./api-utils", () => ({
	apiPost: mocks.apiPost,
	apiDelete: vi.fn(),
	apiFetch: vi.fn(),
}));

import { WebStorageState } from "./storage-state";

describe("web storage listing refresh", () => {
	beforeEach(() => {
		mocks.apiPost.mockReset();
		mocks.apiPost.mockResolvedValue([]);
	});

	test.each([
		["listStorageItems", "apps/app/data/list"],
		["listStorageItemsUser", "apps/app/data/user/list"],
	] as const)(
		"%s refreshes through the URL and preserves the body",
		async (method, endpoint) => {
			const auth = { token: "test" };
			const state = new WebStorageState({ auth } as never);
			for (const options of [undefined, { refresh: false }]) {
				await state[method]("app", "docs", options);
				expect(mocks.apiPost).toHaveBeenLastCalledWith(
					endpoint,
					{ prefix: "docs" },
					auth,
				);
			}
			await state[method]("app", "docs", { refresh: true });
			expect(mocks.apiPost).toHaveBeenLastCalledWith(
				`${endpoint}?refresh=true`,
				{ prefix: "docs" },
				auth,
			);
			mocks.apiPost.mockRejectedValueOnce(new Error("denied"));
			await expect(
				state[method]("app", "docs", { refresh: true }),
			).rejects.toThrow("denied");
		},
	);
});
