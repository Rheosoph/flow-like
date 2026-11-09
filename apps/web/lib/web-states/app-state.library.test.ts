import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiGet: vi.fn() }));

vi.mock("@flow-like/flow-like-ui", () => ({
	IExecutionStage: {},
	ILogLevel: {},
	isAzureBlobStorageUrl: vi.fn(),
}));
vi.mock("./api-utils", () => ({
	apiDelete: vi.fn(),
	apiGet: mocks.apiGet,
	apiPatch: vi.fn(),
	apiPost: vi.fn(),
	apiPut: vi.fn(),
}));

import { WebAppState } from "./app-state";

const auth = { isAuthenticated: true, user: { access_token: "token" } };
const listings = [
	["getApps", "apps?offset=0&limit=100"],
	["getMyGroups", "user/groups"],
] as const;

describe("web library listings", () => {
	beforeEach(() => vi.resetAllMocks());

	test("getApps includes online apps after the first hundred", async () => {
		const apps = Array.from({ length: 203 }, (_, index) => [
			{ id: `app-${index}`, visibility: "Private" },
			{ name: `App ${index}` },
		]);
		mocks.apiGet.mockImplementation(async (path: string) => {
			const params = new URL(path, "https://hub.example").searchParams;
			const offset = Number(params.get("offset") ?? 0);
			const limit = Number(params.get("limit") ?? 100);
			return apps.slice(offset, offset + limit);
		});
		const state = new WebAppState({ auth } as never);

		await expect(state.getApps()).resolves.toEqual(apps);
		expect(mocks.apiGet.mock.calls).toEqual([
			["apps?offset=0&limit=100", auth],
			["apps?offset=100&limit=100", auth],
			["apps?offset=200&limit=100", auth],
		]);
	});

	test.each(listings)(
		"%s preserves request failures for retry",
		async (method, path) => {
			const failure = new Error("session is not ready");
			mocks.apiGet.mockRejectedValue(failure);
			const state = new WebAppState({ auth } as never);

			await expect(state[method]()).rejects.toBe(failure);
			expect(mocks.apiGet).toHaveBeenCalledWith(path, auth);
		},
	);

	test.each(listings)(
		"%s accepts an empty list confirmed by the server",
		async (method, path) => {
			mocks.apiGet.mockResolvedValue([]);
			const state = new WebAppState({ auth } as never);

			await expect(state[method]()).resolves.toEqual([]);
			expect(mocks.apiGet).toHaveBeenCalledWith(path, auth);
		},
	);

	test.each(listings)(
		"%s keeps the last successful query result when refresh fails",
		async (method) => {
			const client = new QueryClient();
			const previous =
				method === "getApps"
					? [[{ id: "saved-app" }, { name: "Saved app" }]]
					: [{ id: "saved-group", name: "Saved group" }];
			const failure = new Error("hub unavailable");
			client.setQueryData([method], previous);
			mocks.apiGet.mockRejectedValue(failure);
			const state = new WebAppState({ auth } as never);

			try {
				await expect(
					client.fetchQuery<unknown>({
						queryKey: [method],
						queryFn: () => state[method](),
						retry: false,
					}),
				).rejects.toBe(failure);
				expect(client.getQueryData([method])).toEqual(previous);
			} finally {
				client.clear();
			}
		},
	);
});
