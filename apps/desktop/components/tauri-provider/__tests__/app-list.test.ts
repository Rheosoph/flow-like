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
	return [{ id, visibility, bits: [] }, { name: id }] as unknown as AppEntry;
}

function signedIcon(credential: string, signedAt: number): string {
	const stamp = new Date(signedAt)
		.toISOString()
		.replace(/[-:]/g, "")
		.replace(/\.\d{3}/, "");
	return `https://media.example/apps/cloud/media/icon.webp?X-Amz-Credential=${credential}&X-Amz-Date=${stamp}&X-Amz-Expires=3600&X-Amz-Signature=sig`;
}

function withIcon(id: string, icon: string): AppEntry {
	return [
		{ id, visibility: IAppVisibility.Private, bits: [] },
		{ name: id, icon },
	] as unknown as AppEntry;
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

	test.each([
		["keeps the live artwork it shows", Date.now(), "shown"],
		[
			"does not adopt shown artwork that has expired",
			Date.UTC(2021, 0, 1),
			"local",
		],
	] as const)(
		"%s while the remote sync runs",
		async (_, shownSignedAt, expected) => {
			const local = withIcon("cloud", signedIcon("old", Date.UTC(2020, 0, 1)));
			const shown = withIcon("cloud", signedIcon("new", shownSignedAt));
			const sync = deferred();
			mocks.fetcher.mockImplementation(() => sync.promise.then(() => [shown]));
			mocks.invoke.mockImplementation((command) =>
				Promise.resolve(command === "get_apps" ? [local] : undefined),
			);
			const { state, client, background } = fixture();
			client.setQueryData(["getApps"], [shown]);
			try {
				const [[, meta]] = await state.getApps();
				expect(meta?.icon).toBe(
					expected === "shown" ? shown[1]?.icon : local[1]?.icon,
				);
			} finally {
				sync.resolve();
				await Promise.all(background);
				client.clear();
			}
		},
	);

	test("rewrites a local copy only when its server record changed", async () => {
		const local = [entry("a"), entry("b")];
		let remote: AppEntry[] = [entry("a"), entry("b")];
		mocks.fetcher.mockImplementation(() => Promise.resolve(remote));
		mocks.invoke.mockImplementation((command) =>
			Promise.resolve(command === "get_apps" ? local : undefined),
		);
		const { state, client, background } = fixture();
		const writes = () =>
			mocks.invoke.mock.calls.filter(([command]) =>
				["update_app", "push_app_meta"].includes(command as string),
			).length;
		const list = async () => {
			await client.fetchQuery({
				queryKey: ["getApps"],
				queryFn: () => state.getApps(),
				staleTime: 0,
			});
			await vi.waitFor(() => expect(background.length).toBeGreaterThan(0));
			await Promise.all(background.splice(0));
			await Promise.all(background.splice(0));
		};
		try {
			await list();
			expect(writes()).toBe(4);
			await list();
			await list();
			expect(mocks.fetcher).toHaveBeenCalledTimes(3);
			expect(writes()).toBe(4);
			expect(mocks.visibilityPut).toHaveBeenCalledTimes(2);

			remote = [entry("a", IAppVisibility.Public), entry("b")];
			await list();
			expect(writes()).toBe(6);
			expect(mocks.invoke).toHaveBeenLastCalledWith(
				"push_app_meta",
				expect.objectContaining({ appId: "a", preserveUpdatedAt: true }),
			);
		} finally {
			await Promise.all(background);
			client.clear();
		}
	});
});
