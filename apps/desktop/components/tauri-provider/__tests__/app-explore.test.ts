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

import { ExploreUnsupportedError } from "@flow-like/flow-like-ui/components/store/explore/explore-types";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { WebAppState } from "../../../../web/lib/web-states/app-state";
import type { TauriBackend } from "../../tauri-provider";
import { AppState } from "../app-state";

const auth = { isAuthenticated: true, user: { access_token: "token" } };
const page = { views: { all: { grid: {}, rows: [] } } };

const adapters = [
	{
		platform: "desktop",
		create: () =>
			new AppState({
				profile: { hub: "hub.example" },
				auth,
			} as unknown as TauriBackend),
		request: mocks.fetcher,
		pathOf: (call: unknown[]) => call[1] as string,
	},
	{
		platform: "web",
		create: () => new WebAppState({ auth } as never),
		request: mocks.apiGet,
		pathOf: (call: unknown[]) => call[0] as string,
	},
];

beforeEach(() => {
	vi.resetAllMocks();
});

for (const { platform, create, request, pathOf } of adapters) {
	describe(`${platform} explore`, () => {
		const requestedParams = () => {
			const path = pathOf(request.mock.calls[0]);
			return new URLSearchParams(path.slice(path.indexOf("?") + 1));
		};

		test("the landing names its own platform and parses the page", async () => {
			request.mockResolvedValue(page);

			const resolved = await create().getExplore({ language: "de", dev: true });

			expect(pathOf(request.mock.calls[0])).toBe(
				`store/explore?platform=${platform}&language=de&dev=true`,
			);
			expect(resolved.views.all).toEqual({
				grid: {
					hero: null,
					notice: null,
					feature: null,
					collection: null,
					stat: null,
					categories: null,
				},
				rows: [],
			});
		});

		test("search sends each list as one comma-joined param", async () => {
			request.mockResolvedValue({ query: "invoice" });

			await create().searchExplore({
				language: "en",
				dev: false,
				q: "invoice",
				categories: ["app:Finance", "package:EDUCATION"],
				permissions: ["network", "models"],
			});

			expect(
				pathOf(request.mock.calls[0]).startsWith("store/explore/search?"),
			).toBe(true);
			const params = requestedParams();
			expect(params.get("platform")).toBe(platform);
			expect(params.getAll("categories")).toEqual([
				"app:Finance,package:EDUCATION",
			]);
			expect(params.getAll("permissions")).toEqual(["network,models"]);
		});

		test("a codeless 404 means the hub has no Explore", async () => {
			request.mockRejectedValue(
				new ApiResponseError({ status: 404, message: "Not Found" }),
			);

			await expect(
				create().getExplore({ language: "en", dev: false }),
			).rejects.toBeInstanceOf(ExploreUnsupportedError);
			await expect(
				create().searchExplore({ language: "en", dev: false }),
			).rejects.toBeInstanceOf(ExploreUnsupportedError);
		});

		test("a coded 404 and other failures pass through", async () => {
			for (const failure of [
				new ApiResponseError({
					status: 404,
					code: "NOT_FOUND",
					message: "Collection not found",
				}),
				new ApiResponseError({ status: 401, message: "Unauthorized" }),
				new Error("Network unavailable: GET /store/explore"),
			]) {
				request.mockRejectedValueOnce(failure);
				await expect(
					create().searchExplore({
						language: "en",
						dev: false,
						collection: "c1",
					}),
				).rejects.toBe(failure);
			}
		});

		test("a body that is not a page is an error, not an empty page", async () => {
			request.mockResolvedValue("<!doctype html><html></html>");

			await expect(
				create().getExplore({ language: "en", dev: false }),
			).rejects.toThrow("Unexpected Explore response");
		});
	});
}

test("desktop Explore needs a hub connection before it requests anything", async () => {
	const state = new AppState({} as TauriBackend);

	await expect(
		state.getExplore({ language: "en", dev: false }),
	).rejects.toThrow("Explore needs a hub connection");
	await expect(
		state.searchExplore({ language: "en", dev: false }),
	).rejects.toThrow("Explore needs a hub connection");
	expect(mocks.fetcher).not.toHaveBeenCalled();
});
