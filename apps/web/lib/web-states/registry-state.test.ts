import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
	apiPost: vi.fn(),
}));

vi.mock("./api-utils", () => ({
	apiDelete: vi.fn(),
	apiGet: mocks.apiGet,
	apiPost: mocks.apiPost,
	apiPut: vi.fn(),
}));

vi.mock(
	"@flow-like/flow-like-ui/components/a2ui/use-micro-widget-grant",
	() => ({ forgetMicroWidgetGrants: vi.fn() }),
);

import { WebRegistryState } from "./registry-state";

const auth = { user: { access_token: "token" } };
const registry = () => new WebRegistryState({ auth } as never);
const ACCESS = "eyJhbGciOiJFUzI1NiJ9.eyJwa2ciOiJ4In0.YWNjZXNz";

const manifest = {
	id: "com.example.sales",
	name: "Sales",
	version: "1.2.0",
	widgets: [{ id: "chart" }],
	widget_bundle_hash: "ab".repeat(32),
};

describe("web registry packages", () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	test("reads the registry entry of the version the project pins", async () => {
		mocks.apiGet.mockResolvedValue({
			id: "com.example.sales",
			manifest,
			source: { type: "registry" },
			updatedAt: "2026-09-30T10:00:00Z",
		});

		await expect(
			registry().getPackage("com.example.sales", "app 1"),
		).resolves.toEqual({
			id: "com.example.sales",
			version: "1.2.0",
			source: { type: "registry" },
			installedAt: "2026-09-30T10:00:00Z",
			wasmPath: "",
			manifest,
		});
		expect(mocks.apiGet).toHaveBeenLastCalledWith(
			"registry/package/com.example.sales?app_id=app%201",
			auth,
		);
	});

	test("without a project nothing is installed, so the store keeps its sign-in and retry states", async () => {
		mocks.apiGet.mockResolvedValue({
			id: "com.example.sales",
			manifest,
			source: { type: "registry" },
			updatedAt: "2026-09-30T10:00:00Z",
		});

		await expect(
			registry().getPackage("com.example.sales"),
		).resolves.toBeNull();
		await expect(
			registry().getPackage("com.example.sales", ""),
		).resolves.toBeNull();
		expect(mocks.apiGet).not.toHaveBeenCalled();
	});

	test("a package the viewer cannot read resolves to nothing", async () => {
		mocks.apiGet.mockRejectedValueOnce(
			new ApiResponseError({ status: 403, message: "Forbidden" }),
		);
		await expect(
			registry().getPackage("com.example.sales", "app-1"),
		).resolves.toBeNull();
		mocks.apiGet.mockResolvedValueOnce(undefined);
		await expect(
			registry().getPackage("com.example.sales", "app-1"),
		).resolves.toBeNull();
	});
});

describe("web widget sandbox", () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	test("the declared describe authorizes through the project", async () => {
		mocks.apiGet.mockRejectedValue(new Error("stop"));
		const request = {
			packageId: "com.example.sales",
			packageVersion: "1.2.0",
			widgetId: "chart",
			preview: false,
		};
		await expect(
			registry().describeWidgetPolicy({ ...request, appId: "app 1" }),
		).rejects.toThrow("stop");
		expect(mocks.apiGet).toHaveBeenLastCalledWith(
			"registry/package/com.example.sales/widget-policy/1.2.0/chart?preview=false&app_id=app%201",
			auth,
		);
		await expect(registry().describeWidgetPolicy(request)).rejects.toThrow(
			"stop",
		);
		expect(mocks.apiGet).toHaveBeenLastCalledWith(
			"registry/package/com.example.sales/widget-policy/1.2.0/chart?preview=false",
			auth,
		);
	});

	test("asks for an access token of the version, through the project when one is given", async () => {
		mocks.apiPost.mockResolvedValue({ access: ACCESS, expiresIn: 43_200 });
		await expect(
			registry().getWidgetAccess({
				packageId: "com.example.sales",
				packageVersion: "1.2.0",
				appId: "app-1",
			}),
		).resolves.toEqual({ access: ACCESS, expiresIn: 43_200 });
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
			"registry/package/com.example.sales/widget-access",
			{ version: "1.2.0", appId: "app-1" },
			auth,
		);

		mocks.apiPost.mockResolvedValue({ access: null, expiresIn: 43_200 });
		await expect(
			registry().getWidgetAccess({
				packageId: "com.example.sales",
				packageVersion: "1.2.0",
			}),
		).resolves.toEqual({ access: null, expiresIn: 43_200 });
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
			"registry/package/com.example.sales/widget-access",
			{ version: "1.2.0" },
			auth,
		);
	});

	test("an API without the route loads anonymously; a refusal or a malformed token rejects", async () => {
		const request = { packageId: "com.example.sales", packageVersion: "1.2.0" };
		mocks.apiPost.mockRejectedValueOnce(
			new ApiResponseError({ status: 405, message: "Method Not Allowed" }),
		);
		await expect(registry().getWidgetAccess(request)).resolves.toMatchObject({
			access: null,
		});

		const refusal = new ApiResponseError({
			status: 404,
			code: "NOT_FOUND",
			message: "Package not found",
		});
		mocks.apiPost.mockRejectedValueOnce(refusal);
		await expect(registry().getWidgetAccess(request)).rejects.toBe(refusal);

		mocks.apiPost.mockResolvedValueOnce({ access: "a/b", expiresIn: 60 });
		await expect(registry().getWidgetAccess(request)).rejects.toThrow(
			"malformed access token",
		);
	});
});
