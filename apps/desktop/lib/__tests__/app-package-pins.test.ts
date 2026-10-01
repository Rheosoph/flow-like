import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));

vi.mock("../api", () => ({
	fetcher: mocks.fetcher,
	put: vi.fn(),
}));

vi.mock("../apps-db", () => ({
	appsDB: {
		visibility: { get: vi.fn(), put: vi.fn() },
	},
}));

import { AppState } from "../../components/tauri-provider/app-state";

const HUB_PINS = [
	{ packageId: "pkg-active", version: "1.0.0" },
	{
		packageId: "pkg-expired",
		version: "3.0.0",
		license: { required: true, status: "expired", graceDays: 30 },
	},
];

/** The native pin store of one app: the commands the app state invokes. */
function localPins(initial: Record<string, string>) {
	const pins = { ...initial };
	mocks.invoke.mockImplementation(
		async (
			command: string,
			args: { packageId?: string; version?: string },
		) => {
			if (command === "app_list_packages") return { ...pins };
			if (command === "app_add_package" && args.packageId && args.version) {
				pins[args.packageId] = args.version;
			}
			if (command === "app_remove_package" && args.packageId) {
				delete pins[args.packageId];
			}
			return undefined;
		},
	);
	return pins;
}

function appState({
	signedIn = true,
	offline = false,
	queryClient = undefined as QueryClient | undefined,
} = {}) {
	const backend: Record<string, unknown> = {
		profile: signedIn ? { id: "profile-1", hub: "hub.example" } : undefined,
		auth: signedIn
			? { isAuthenticated: true, user: { access_token: "token" } }
			: undefined,
		isOffline: vi.fn().mockResolvedValue(offline),
		queryClient,
	};
	const state = new AppState(backend as never);
	backend.appState = state;
	return state;
}

const writes = () =>
	mocks.invoke.mock.calls.filter(
		([command]) => command !== "app_list_packages",
	);

describe("desktop project package pins", () => {
	beforeEach(() => {
		mocks.invoke.mockReset();
		mocks.fetcher.mockReset();
	});

	test("an online project's local pins take what the hub pins, without expired licences", async () => {
		const pins = localPins({ "pkg-local": "0.1.0", "pkg-active": "0.9.0" });
		mocks.fetcher.mockResolvedValue(HUB_PINS);

		await expect(appState().listPackages("app-1")).resolves.toEqual({
			"pkg-active": "1.0.0",
		});
		expect(mocks.fetcher.mock.calls[0][1]).toBe("apps/app-1/packages");
		expect(writes()).toEqual([
			[
				"app_add_package",
				{ appId: "app-1", packageId: "pkg-active", version: "1.0.0" },
			],
			["app_remove_package", { appId: "app-1", packageId: "pkg-local" }],
		]);
		expect(pins).toEqual({ "pkg-active": "1.0.0" });
	});

	test("pins that already match the hub are left alone", async () => {
		localPins({ "pkg-active": "1.0.0" });
		mocks.fetcher.mockResolvedValue(HUB_PINS);

		await expect(appState().listPackages("app-1")).resolves.toEqual({
			"pkg-active": "1.0.0",
		});
		expect(writes()).toEqual([]);
	});

	test("offline, signed out or with the hub unreachable it lists the local pins untouched", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		const local = { "pkg-local": "0.1.0" };
		localPins(local);

		await expect(
			appState({ offline: true }).listPackages("app-1"),
		).resolves.toEqual(local);
		await expect(
			appState({ signedIn: false }).listPackages("app-1"),
		).resolves.toEqual(local);
		expect(mocks.fetcher).not.toHaveBeenCalled();

		mocks.fetcher.mockRejectedValue(new Error("hub unavailable"));
		await expect(appState().listPackages("app-1")).resolves.toEqual(local);
		mocks.fetcher.mockResolvedValue({ error: "not a list" });
		await expect(appState().listPackages("app-1")).resolves.toEqual(local);

		expect(writes()).toEqual([]);
		expect(warn).toHaveBeenCalledTimes(2);
		warn.mockRestore();
	});

	test("listings at the same time share one hub sync", async () => {
		localPins({});
		mocks.fetcher.mockResolvedValue(HUB_PINS);
		const state = appState();

		const listings = await Promise.all([
			state.listPackages("app-1"),
			state.listPackages("app-1"),
		]);
		expect(listings).toEqual([
			{ "pkg-active": "1.0.0" },
			{ "pkg-active": "1.0.0" },
		]);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
	});

	test("the hub is asked again only after a pin change invalidates the app's packages", async () => {
		localPins({});
		mocks.fetcher.mockResolvedValue(HUB_PINS);
		const queryClient = new QueryClient();
		const state = appState({ queryClient });

		await state.listPackages("app-1");
		await state.listPackages("app-1");
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);

		mocks.fetcher.mockResolvedValue([
			{ packageId: "pkg-active", version: "1.1.0" },
		]);
		await queryClient.invalidateQueries({
			queryKey: ["app", "app-1", "packages"],
		});
		await expect(state.listPackages("app-1")).resolves.toEqual({
			"pkg-active": "1.1.0",
		});
		expect(mocks.fetcher).toHaveBeenCalledTimes(2);
		queryClient.clear();
	});
});
