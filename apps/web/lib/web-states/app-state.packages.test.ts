import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
}));

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

const auth = { user: { access_token: "token" } };

describe("web app packages", () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	test("lists the versions the project pins, without expired licences", async () => {
		mocks.apiGet.mockResolvedValue([
			{ packageId: "pkg-active", version: "1.0.0" },
			{
				packageId: "pkg-lapsed",
				version: "2.0.0",
				license: { required: true, status: "lapsed", graceDays: 30 },
			},
			{
				packageId: "pkg-expired",
				version: "3.0.0",
				license: { required: true, status: "expired", graceDays: 30 },
			},
		]);

		await expect(
			new WebAppState({ auth } as never).listPackages("app-1"),
		).resolves.toEqual({ "pkg-active": "1.0.0", "pkg-lapsed": "2.0.0" });
		expect(mocks.apiGet).toHaveBeenCalledWith("apps/app-1/packages", auth);
	});

	test("a failed listing rejects instead of reporting a project without packages", async () => {
		const failure = new Error("hub unavailable");
		mocks.apiGet.mockRejectedValue(failure);
		await expect(
			new WebAppState({ auth } as never).listPackages("app-1"),
		).rejects.toBe(failure);
	});
});
