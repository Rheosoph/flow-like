import type { IApp, IMetadata } from "@flow-like/flow-like-ui";
import { IAppVisibility } from "@flow-like/flow-like-ui/lib/schema/app/app";
import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	visibilityPut: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));

vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	put: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await vi.importActual<Record<string, unknown>>(
		"@flow-like/flow-like-ui/lib/schema/app/app",
	)),
	IExecutionStage: { Dev: "Dev" },
	ILogLevel: { Debug: "Debug" },
	injectDataFunction: (
		await vi.importActual<
			typeof import("@flow-like/flow-like-ui/hooks/use-invoke")
		>("@flow-like/flow-like-ui/hooks/use-invoke")
	).injectDataFunction,
	discardOfflineSyncForApp: vi.fn(),
}));

vi.mock("../../../lib/apps-db", () => ({
	appsDB: { visibility: { put: mocks.visibilityPut } },
}));

import type { TauriBackend } from "../../tauri-provider";
import { AppState } from "../app-state";

type AppEntry = [IApp, IMetadata | undefined];

function entry(id: string, visibility = IAppVisibility.Private): AppEntry {
	return [{ id, visibility, bits: [] }, { name: id }] as AppEntry;
}

function deferred() {
	let resolve!: () => void;
	const promise = new Promise<void>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

function fixture() {
	const client = new QueryClient();
	const background: Promise<unknown>[] = [];
	const backend = {
		profile: { hub: "hub.example" },
		auth: { isAuthenticated: true, user: { access_token: "token" } },
		queryClient: client,
		backgroundTaskHandler: (promise: Promise<unknown>) =>
			background.push(promise),
	} as unknown as TauriBackend;
	return { client, background, state: new AppState(backend) };
}

beforeEach(() => {
	vi.resetAllMocks();
	mocks.visibilityPut.mockResolvedValue(undefined);
});

afterEach(() => {
	vi.restoreAllMocks();
});

describe("desktop app listing", () => {
	test("publishes all online apps while native cache creation is still pending", async () => {
		const cached = entry("cached");
		const remote = [cached, entry("new-a"), entry("new-b")];
		const cacheWrite = deferred();
		mocks.fetcher.mockResolvedValue(remote);
		mocks.invoke.mockImplementation((command) => {
			if (command === "get_apps") return Promise.resolve([cached]);
			if (command === "create_app") return cacheWrite.promise;
			return Promise.resolve();
		});
		const { state, client, background } = fixture();
		try {
			await client.fetchQuery({
				queryKey: ["getApps"],
				queryFn: () => state.getApps(),
			});
			await vi.waitFor(
				() => {
					expect(client.getQueryData(["getApps"])).toEqual(remote);
				},
				{ timeout: 250 },
			);
			expect(mocks.invoke).toHaveBeenCalledWith(
				"create_app",
				expect.objectContaining({ id: "new-a", addToProfile: false }),
			);
		} finally {
			cacheWrite.resolve();
			await Promise.all(background);
			client.clear();
		}
	});

	test("returns the online library on a new device before native caching finishes", async () => {
		const remote = [entry("new-a"), entry("new-b")];
		const cacheWrite = deferred();
		mocks.fetcher.mockResolvedValue(remote);
		mocks.invoke.mockImplementation((command) => {
			if (command === "get_apps") return Promise.resolve([]);
			if (command === "create_app") return cacheWrite.promise;
			return Promise.resolve();
		});
		const { state, client, background } = fixture();
		let listed: AppEntry[] | undefined;
		const read = state.getApps().then((apps) => {
			listed = apps;
		});
		try {
			await vi.waitFor(() => expect(listed).toEqual(remote), { timeout: 250 });
		} finally {
			cacheWrite.resolve();
			await read;
			await Promise.all(background);
			client.clear();
		}
	});

	test("keeps online apps visible when a native cache write fails", async () => {
		const remote = [entry("new-a"), entry("new-b")];
		mocks.fetcher.mockResolvedValue(remote);
		mocks.invoke.mockImplementation((command) => {
			if (command === "get_apps") return Promise.resolve([]);
			if (command === "create_app")
				return Promise.reject(new Error("Disk full"));
			return Promise.resolve();
		});
		const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
		const { state, client, background } = fixture();
		await expect(state.getApps()).resolves.toEqual(remote);
		await Promise.all(background);
		expect(warning).toHaveBeenCalled();
		client.clear();
	});
});
