import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	fetcher: vi.fn(),
	apiGet: vi.fn(),
}));

vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	put: vi.fn(),
}));

vi.mock("../../../../web/lib/web-states/api-utils", () => ({
	apiGet: mocks.apiGet,
	apiDelete: vi.fn(),
	apiPatch: vi.fn(),
	apiPost: vi.fn(),
	apiPut: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await vi.importActual<Record<string, unknown>>(
		"@flow-like/flow-like-ui/lib/schema/app/app",
	)),
	IExecutionStage: { Dev: "Dev" },
	ILogLevel: { Debug: "Debug" },
	injectDataFunction: vi.fn(),
	discardOfflineSyncForApp: vi.fn(),
	isAzureBlobStorageUrl: vi.fn(),
}));

vi.mock("../../../lib/apps-db", () => ({
	appsDB: { visibility: { get: vi.fn(), put: vi.fn() } },
}));

import { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import { WebAppState } from "../../../../web/lib/web-states/app-state";
import type { TauriBackend } from "../../tauri-provider";
import { AppState } from "../app-state";

function desktopBackend() {
	return {
		profile: { hub: "hub.example" },
		auth: { isAuthenticated: true, user: { access_token: "token" } },
	} as unknown as TauriBackend;
}

const adapters = [
	{
		name: "desktop",
		create: () => new AppState(desktopBackend()),
		request: mocks.fetcher,
	},
	{
		name: "web",
		create: () => new WebAppState({ auth: undefined } as never),
		request: mocks.apiGet,
	},
];

beforeEach(() => {
	vi.resetAllMocks();
});

for (const { name, create, request } of adapters) {
	describe(`${name} app search`, () => {
		const search = (state: ReturnType<typeof create>, offset = 0) =>
			state.searchApps(
				undefined,
				"calendar",
				undefined,
				undefined,
				undefined,
				IAppSearchSort.MostPopular,
				undefined,
				offset,
				50,
			);

		test.each([0, 50])(
			"exposes a failed page at offset %i so the query can be retried",
			async (offset) => {
				const failure = new Error("The store is unavailable");
				request.mockRejectedValue(failure);

				await expect(search(create(), offset)).rejects.toBe(failure);
			},
		);

		test("keeps a successful empty response distinct from a failure", async () => {
			request.mockResolvedValue([]);

			await expect(search(create())).resolves.toEqual([]);
		});

		test("returns search results after retrying a failed request", async () => {
			const entries = [[{ id: "calendar-app" }, undefined]];
			const state = create();
			request
				.mockRejectedValueOnce(new Error("The store is unavailable"))
				.mockResolvedValueOnce(entries);

			await expect(search(state)).rejects.toThrow("The store is unavailable");
			await expect(search(state)).resolves.toEqual(entries);
		});

		test("retains the local library fallback when no search is requested", async () => {
			const state = create();
			const getApps = vi.spyOn(state, "getApps").mockResolvedValue([]);

			await expect(state.searchApps()).resolves.toEqual([]);
			expect(getApps).toHaveBeenCalledOnce();
			expect(request).not.toHaveBeenCalled();
		});
	});
}

test("desktop search stays empty until a profile is available", async () => {
	const state = new AppState({} as TauriBackend);

	await expect(state.searchApps(undefined, "calendar")).resolves.toEqual([]);
	expect(mocks.fetcher).not.toHaveBeenCalled();
});
