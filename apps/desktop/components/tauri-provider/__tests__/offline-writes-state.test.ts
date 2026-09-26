import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn(),
	fetcher: vi.fn(),
	put: vi.fn(),
	discardOfflineSyncForApp: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	put: mocks.put,
}));

// The barrel drags the whole component library into the module graph; these
// suites only need the enums and helpers `app-state` reads at runtime.
vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await vi.importActual<Record<string, unknown>>(
		"@flow-like/flow-like-ui/lib/schema/app/app",
	)),
	IExecutionStage: { Dev: "Dev" },
	ILogLevel: { Debug: "Debug" },
	injectDataFunction: vi.fn(),
	discardOfflineSyncForApp: mocks.discardOfflineSyncForApp,
}));

vi.mock("../../../lib/apps-db", () => ({
	appsDB: {
		visibility: { get: vi.fn(), put: vi.fn() },
	},
}));

import type { IOfflineWritesState } from "@flow-like/flow-like-ui/state/backend-state/offline-writes-state";
import type { TauriBackend } from "../../tauri-provider";
import { AppState } from "../app-state";
import { DatabaseState } from "../db-state";
import { OfflineWritesState } from "../offline-writes-state";

const APP = "app-1";
const TOKEN = "token-1";
const STRUCTURE_BLOCKED =
	"Turn off offline access for this table before changing its structure.";

function backend(overrides: Record<string, unknown> = {}) {
	return {
		auth: { user: { access_token: TOKEN, profile: { sub: "user-7" } } },
		prepareExecutionAuth: vi.fn().mockResolvedValue("https://hub.example"),
		executionSessionId: "session-1",
		...overrides,
	} as unknown as TauriBackend;
}

function signedOut() {
	return backend({ auth: undefined });
}

beforeEach(() => {
	mocks.invoke.mockReset().mockResolvedValue(undefined);
	mocks.listen.mockReset();
	mocks.fetcher.mockReset().mockResolvedValue([]);
	mocks.discardOfflineSyncForApp.mockReset().mockResolvedValue(0);
});

describe("OfflineWritesState commands", () => {
	test("the overview passes the token, or null when signed out", async () => {
		await new OfflineWritesState(backend()).getOverview(APP);
		await new OfflineWritesState(signedOut()).getOverview(APP);
		expect(mocks.invoke.mock.calls).toEqual([
			["offline_writes_overview", { appId: APP, token: TOKEN }],
			["offline_writes_overview", { appId: APP, token: null }],
		]);
	});

	test("turning a table on prepares execution auth first", async () => {
		const host = backend();
		const order: string[] = [];
		vi.mocked(host.prepareExecutionAuth).mockImplementation(async () => {
			order.push("prepare");
			return "https://hub.example";
		});
		mocks.invoke.mockImplementation(async (command: string) => {
			order.push(command);
		});
		const selection = {
			purpose: "storage" as const,
			table: "orders",
			primaryKey: "id",
			prefetch: true,
		};

		await new OfflineWritesState(host).setTable(APP, selection);

		expect(order).toEqual(["prepare", "offline_writes_set_table"]);
		expect(mocks.invoke).toHaveBeenCalledWith("offline_writes_set_table", {
			appId: APP,
			selection,
			token: TOKEN,
			sessionId: "session-1",
		});
	});

	test("Download everything sends purpose, table and the flag", async () => {
		await new OfflineWritesState(backend()).setPrefetch(
			APP,
			"user",
			"notes",
			true,
		);
		expect(mocks.invoke).toHaveBeenCalledWith("offline_writes_set_prefetch", {
			appId: APP,
			token: TOKEN,
			purpose: "user",
			table: "notes",
			prefetch: true,
		});
	});

	test("every command uses the §4.11 name and arguments", async () => {
		const state = new OfflineWritesState(backend());
		const limits = {
			maxQueueBytes: 1,
			maxOperations: 2,
			maxAgeSeconds: 3,
			maxMirrorBytes: 4,
		};
		await state.removeTable(APP, "storage", "orders");
		await state.setLimits(APP, limits);
		await state.listOperations(APP);
		await state.listOperations(APP, 7, 50);
		await state.getOperationState(APP, "op-1");
		await state.retryOperation(APP, "op-1");
		await state.skipOperation(APP, "op-1", "duplicate", true);
		await state.keepBoth(APP, "op-2");
		await state.syncNow(APP);
		await state.forgetApp(APP, true);

		expect(mocks.invoke.mock.calls).toEqual([
			[
				"offline_writes_remove_table",
				{ appId: APP, token: TOKEN, purpose: "storage", table: "orders" },
			],
			["offline_writes_set_limits", { appId: APP, token: TOKEN, limits }],
			[
				"offline_writes_operations",
				{ appId: APP, token: TOKEN, afterSequence: null, limit: null },
			],
			[
				"offline_writes_operations",
				{ appId: APP, token: TOKEN, afterSequence: 7, limit: 50 },
			],
			[
				"offline_writes_operation_state",
				{ appId: APP, token: TOKEN, operationId: "op-1" },
			],
			[
				"offline_writes_retry",
				{ appId: APP, token: TOKEN, operationId: "op-1" },
			],
			[
				"offline_writes_skip",
				{
					appId: APP,
					token: TOKEN,
					operationId: "op-1",
					reason: "duplicate",
					acknowledgeUncertain: true,
				},
			],
			[
				"offline_writes_keep_both",
				{ appId: APP, token: TOKEN, operationId: "op-2" },
			],
			["offline_writes_sync_now", { appId: APP, token: TOKEN }],
			[
				"offline_writes_forget_app",
				{ appId: APP, token: TOKEN, allAccounts: true },
			],
		]);
	});

	test("management commands refuse to run signed out", async () => {
		const state = new OfflineWritesState(signedOut());
		await expect(state.syncNow(APP)).rejects.toThrow(
			"Sign in to manage offline access for this project.",
		);
		await state.forgetApp(APP, false);
		expect(mocks.invoke.mock.calls).toEqual([
			[
				"offline_writes_forget_app",
				{ appId: APP, token: null, allAccounts: false },
			],
		]);
	});

	test("the table route is asked on every call", async () => {
		mocks.invoke.mockResolvedValue("device");
		const state = new OfflineWritesState(backend());
		expect(await state.getTableRoute(APP, "orders")).toBe("device");
		expect(await state.getTableRoute(APP, "notes", true)).toBe("device");
		expect(mocks.invoke.mock.calls).toEqual([
			[
				"offline_writes_table_route",
				{ appId: APP, token: TOKEN, table: "orders", userScoped: false },
			],
			[
				"offline_writes_table_route",
				{ appId: APP, token: TOKEN, table: "notes", userScoped: true },
			],
		]);
	});

	test("without an account no table is configured, so the route is the hub", async () => {
		expect(
			await new OfflineWritesState(signedOut()).getTableRoute(APP, "orders"),
		).toBe("hub");
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});

describe("OfflineWritesState events", () => {
	test.each([
		["subscribe", "offline-writes:status"],
		["subscribeTables", "offline-writes:tables-changed"],
		["subscribeMirror", "offline-writes:mirror"],
	] as const)("%s listens to %s", async (method, name) => {
		const unlisten = vi.fn();
		let handler: ((event: { payload: unknown }) => void) | undefined;
		mocks.listen.mockImplementation(async (_name, callback) => {
			handler = callback;
			return unlisten;
		});
		const listener = vi.fn();

		const stop = (new OfflineWritesState(backend()) as IOfflineWritesState)[
			method
		](listener);
		await vi.waitFor(() => expect(handler).toBeDefined());
		handler?.({ payload: { appId: APP } });
		stop();

		expect(mocks.listen).toHaveBeenCalledWith(name, expect.any(Function));
		expect(listener).toHaveBeenCalledWith({ appId: APP });
		expect(unlisten).toHaveBeenCalledOnce();
	});

	test("unsubscribing before the listener is registered still removes it", async () => {
		const unlisten = vi.fn();
		let resolve: ((stop: () => void) => void) | undefined;
		mocks.listen.mockReturnValue(
			new Promise((done) => {
				resolve = done;
			}),
		);

		const stop = new OfflineWritesState(backend()).subscribeMirror(vi.fn());
		stop();
		resolve?.(unlisten);
		await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
	});
});

describe("Data Studio routing", () => {
	function database(route: "device" | "hub" | undefined, offline = false) {
		const getTableRoute = vi.fn().mockResolvedValue(route);
		const host = backend({
			isOffline: vi.fn().mockResolvedValue(offline),
			profile: { id: "profile-1" },
			offlineWritesState: route ? { getTableRoute } : undefined,
		});
		return { db: new DatabaseState(host), getTableRoute, host };
	}

	test("rows of a device table go to Tauri with the token, asking the route every time", async () => {
		const { db, getTableRoute } = database("device");

		await db.addItems(APP, "orders", [{ id: 1 }]);
		await db.listItems(APP, "orders", 0, 25, true);

		expect(getTableRoute.mock.calls).toEqual([
			[APP, "orders", undefined],
			[APP, "orders", true],
		]);
		expect(mocks.invoke.mock.calls).toEqual([
			[
				"db_add",
				{
					appId: APP,
					tableName: "orders",
					items: [{ id: 1 }],
					userScoped: false,
					token: TOKEN,
				},
			],
			[
				"db_list",
				{
					appId: APP,
					tableName: "orders",
					offset: 0,
					limit: 25,
					userScoped: true,
					token: TOKEN,
				},
			],
		]);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("every row method of a device table reaches its Tauri command", async () => {
		const { db } = database("device");
		await db.removeItems(APP, "orders", "id = 1");
		await db.queryItems(APP, "orders", {} as never);
		await db.countItems(APP, "orders");
		await db.getSchema(APP, "orders");
		await db.updateItem(APP, "orders", "id = 1", { name: "x" });

		expect(
			mocks.invoke.mock.calls.map(([command, args]) => [
				command,
				(args as { token?: string }).token,
			]),
		).toEqual([
			["db_delete", TOKEN],
			["db_query", TOKEN],
			["db_count", TOKEN],
			["db_schema", TOKEN],
			["db_update", TOKEN],
		]);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("structural changes of a device table are blocked", async () => {
		const { db } = database("device");
		const blocked = [
			db.createTable(APP, "orders", []),
			db.buildIndex(APP, "orders", "id", 1),
			db.dropIndex(APP, "orders", "idx"),
			db.optimize(APP, "orders"),
			db.dropColumns(APP, "orders", ["a"]),
			db.addColumn(APP, "orders", { name: "a", sql_expression: "1" }),
			db.alterColumn(APP, "orders", "a", true),
			db.setPrimaryKey(APP, "orders", "id"),
			db.dropTable(APP, "orders"),
			db.databaseAction(APP, "orders", { type: "checkout" } as never),
		];
		for (const call of blocked) {
			await expect(call).rejects.toThrow(STRUCTURE_BLOCKED);
		}
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("hub tables keep the API path", async () => {
		const { db, getTableRoute } = database("hub");
		await db.addItems(APP, "orders", []);
		await db.optimize(APP, "orders");
		expect(getTableRoute).toHaveBeenCalledTimes(2);
		expect(mocks.fetcher).toHaveBeenCalledTimes(2);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("offline apps never ask for a route", async () => {
		const { db, getTableRoute } = database("device", true);
		await db.addItems(APP, "orders", [{ id: 1 }]);
		expect(getTableRoute).not.toHaveBeenCalled();
		expect(mocks.invoke).toHaveBeenCalledWith("db_add", {
			appId: APP,
			tableName: "orders",
			items: [{ id: 1 }],
			userScoped: false,
		});
	});
});

describe("forgetting offline data with the app", () => {
	function appBackend(forgetApp = vi.fn().mockResolvedValue(undefined)) {
		return {
			forgetApp,
			host: backend({
				isOffline: vi.fn().mockResolvedValue(true),
				isLocalOnly: vi.fn().mockResolvedValue(false),
				profile: { hub: "hub.example" },
				queryClient: {},
				offlineWritesState: { forgetApp },
			}),
		};
	}

	test("deleting forgets every account's offline data", async () => {
		const { forgetApp, host } = appBackend();
		await new AppState(host).deleteApp(APP);
		expect(forgetApp).toHaveBeenCalledWith(APP, true);
	});

	test("leaving forgets only this account's offline data", async () => {
		const { forgetApp, host } = appBackend();
		mocks.fetcher.mockResolvedValue(undefined);
		await new AppState(host).leaveApp(APP);
		expect(forgetApp).toHaveBeenCalledWith(APP, false);
	});

	test("a failed forget never fails the delete", async () => {
		const { host } = appBackend(vi.fn().mockRejectedValue(new Error("locked")));
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		await expect(new AppState(host).deleteApp(APP)).resolves.toBeUndefined();
		expect(mocks.invoke).toHaveBeenCalledWith("delete_app", { appId: APP });
		warn.mockRestore();
	});
});
