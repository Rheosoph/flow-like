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
	streamFetcher: vi.fn(),
}));

vi.mock("sonner", () => ({
	toast: Object.assign(vi.fn(), {
		success: vi.fn(),
		error: vi.fn(),
		info: vi.fn(),
		warning: vi.fn(),
		dismiss: vi.fn(),
	}),
}));

import { IExecutionMode } from "@flow-like/flow-like-ui";
import type { IBoardRunRequirements } from "@flow-like/flow-like-ui/state/backend-state/board-state";
import { BoardState } from "../../components/tauri-provider/board-state";
import { EventState } from "../../components/tauri-provider/event-state";

function runRequirements(
	overrides: Partial<IBoardRunRequirements> = {},
): IBoardRunRequirements {
	return {
		runtime_variables: [],
		oauth_requirements: [],
		requires_local_execution: false,
		execution_mode: IExecutionMode.Hybrid,
		wasm_package_ids: [],
		wasm_package_permissions: {},
		instantiates_widgets: false,
		...overrides,
	};
}

function hostedBackend(authenticated = true) {
	return {
		isLocalOnly: vi.fn().mockResolvedValue(false),
		profile: { id: "profile-1", hub: "hub.example" },
		auth: authenticated
			? { isAuthenticated: true, user: { access_token: "token" } }
			: { isAuthenticated: false, user: undefined },
		queryClient: { setQueryData: vi.fn(), invalidateQueries: vi.fn() },
		backgroundTaskHandler: vi.fn(),
	};
}

function localBackend() {
	return {
		...hostedBackend(),
		isLocalOnly: vi.fn().mockResolvedValue(true),
	};
}

describe("local execution widget preparation", () => {
	test("waits for widget hydration even when there are no packages to install", async () => {
		const hydration = Promise.withResolvers<void>();
		const syncWidgetsForExecution = vi.fn(() => hydration.promise);
		const backend = {
			...hostedBackend(),
			widgetState: { syncWidgetsForExecution },
			isOffline: vi.fn().mockResolvedValue(false),
			appState: {},
		};
		const state = new BoardState(backend as never);
		let prepared = false;
		const preparation = state
			.ensureAppPackagesInstalledForExecution(
				"app-1",
				runRequirements({ instantiates_widgets: true }),
			)
			.then(() => {
				prepared = true;
			});

		await Promise.resolve();
		expect(syncWidgetsForExecution).toHaveBeenCalledWith("app-1");
		expect(prepared).toBe(false);
		hydration.resolve();
		await preparation;
		expect(prepared).toBe(true);
	});

	test("rejects local execution preparation when widget hydration fails", async () => {
		const failure = new Error("Widget cache could not be written");
		const backend = {
			...hostedBackend(),
			widgetState: {
				syncWidgetsForExecution: vi.fn().mockRejectedValue(failure),
			},
			isOffline: vi.fn().mockResolvedValue(false),
			appState: {},
		};
		const state = new BoardState(backend as never);

		await expect(
			state.ensureAppPackagesInstalledForExecution(
				"app-1",
				runRequirements({ instantiates_widgets: true }),
			),
		).rejects.toBe(failure);
	});

	test("does not require widget access for unrelated boards or package-only callers", async () => {
		const syncWidgetsForExecution = vi
			.fn()
			.mockRejectedValue(new Error("ReadWidgets denied"));
		const backend = {
			...hostedBackend(),
			widgetState: { syncWidgetsForExecution },
			isOffline: vi.fn().mockResolvedValue(false),
			appState: {},
		};
		const state = new BoardState(backend as never);

		await state.ensureAppPackagesInstalledForExecution(
			"app-1",
			runRequirements(),
		);
		await state.ensureAppPackagesInstalledForExecution("app-1");
		expect(syncWidgetsForExecution).not.toHaveBeenCalled();
	});
});

describe("project package licences before a local run", () => {
	const pin = (
		packageId: string,
		version: string,
		status: "active" | "lapsed" | "expired",
	) => ({
		packageId,
		version,
		packageName: `Package ${packageId}`,
		stale: status !== "active",
		license: { required: true, status, graceDays: 30 },
	});

	function licensedBackend(localPackages: Record<string, string> = {}) {
		mocks.fetcher.mockResolvedValueOnce([
			pin("pkg-active", "1.0.0", "active"),
			pin("pkg-lapsed", "2.0.0", "lapsed"),
			pin("pkg-expired", "3.0.0", "expired"),
		]);
		const appState = {
			listPackages: vi.fn().mockResolvedValue(localPackages),
			addPackage: vi.fn().mockResolvedValue(undefined),
			removePackage: vi.fn().mockResolvedValue(undefined),
		};
		const registryState = {
			getInstalledPackages: vi.fn().mockResolvedValue([]),
			installPackage: vi.fn().mockResolvedValue({}),
		};
		const backend = {
			...hostedBackend(),
			widgetState: {},
			isOffline: vi.fn().mockResolvedValue(false),
			appState,
			registryState,
		};
		return { state: new BoardState(backend as never), appState, registryState };
	}

	beforeEach(() => {
		vi.clearAllMocks();
		mocks.fetcher.mockReset();
	});

	test("installs usable pins through the project licence and skips expired ones", async () => {
		const { state, appState, registryState } = licensedBackend({
			"pkg-expired": "3.0.0",
		});

		await state.ensureAppPackagesInstalledForExecution(
			"app-1",
			runRequirements(),
		);

		expect(registryState.installPackage).toHaveBeenCalledTimes(2);
		expect(registryState.installPackage).toHaveBeenCalledWith(
			"pkg-active",
			"1.0.0",
			undefined,
			"app-1",
		);
		expect(registryState.installPackage).toHaveBeenCalledWith(
			"pkg-lapsed",
			"2.0.0",
			undefined,
			"app-1",
		);
		expect(appState.addPackage).toHaveBeenCalledTimes(2);
		expect(appState.addPackage).not.toHaveBeenCalledWith(
			"app-1",
			"pkg-expired",
			expect.anything(),
		);
		expect(appState.removePackage).toHaveBeenCalledWith("app-1", "pkg-expired");
	});

	test("refuses a local run of a board that uses an expired package", async () => {
		const { state, registryState } = licensedBackend();

		await expect(
			state.ensureAppPackagesInstalledForExecution(
				"app-1",
				runRequirements({ wasm_package_ids: ["pkg-expired"] }),
			),
		).rejects.toThrow(
			"Package pkg-expired is disabled in this project: its licence expired",
		);
		expect(registryState.installPackage).not.toHaveBeenCalled();
	});

	test("lets boards run that only use lapsed packages", async () => {
		const { state, registryState } = licensedBackend();

		await state.ensureAppPackagesInstalledForExecution(
			"app-1",
			runRequirements({ wasm_package_ids: ["pkg-lapsed"] }),
		);
		expect(registryState.installPackage).toHaveBeenCalledTimes(2);
	});
});

describe("authoritative Board and Event reads", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test("propagates a hosted Board inventory failure without reading local storage", async () => {
		const failure = new Error("hub unavailable");
		mocks.fetcher.mockRejectedValueOnce(failure);
		const state = new BoardState(hostedBackend() as never);

		await expect(
			state.getBoardSummariesAuthoritative("app-1", ["node_types"]),
		).rejects.toBe(failure);
		expect(mocks.fetcher).toHaveBeenCalledWith(
			{ id: "profile-1", hub: "hub.example" },
			"apps/app-1/board/summaries?include=node_types",
			{ method: "GET" },
			expect.objectContaining({ isAuthenticated: true }),
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("reads an exact hosted Board revision without cache or native writes", async () => {
		const board = { id: "board-1", nodes: {} };
		const backend = hostedBackend();
		mocks.fetcher.mockResolvedValueOnce(board);
		const state = new BoardState(backend as never);

		await expect(
			state.getBoardAuthoritative("app-1", "board-1", [2, 1, 0]),
		).resolves.toBe(board);
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app-1/board/board-1?version=2_1_0",
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(backend.queryClient.setQueryData).not.toHaveBeenCalled();
	});

	test("reads local-only Board and FlowScript revisions through native storage", async () => {
		const board = { id: "board-1", nodes: {} };
		mocks.invoke
			.mockResolvedValueOnce(board)
			.mockResolvedValueOnce("event entry() {}\n");
		const state = new BoardState(localBackend() as never);

		await expect(
			state.getBoardAuthoritative("app-1", "board-1", [3, 0, 0]),
		).resolves.toBe(board);
		await expect(
			state.getFlowScriptAuthoritative("app-1", "board-1", [3, 0, 0]),
		).resolves.toBe("event entry() {}\n");
		expect(mocks.invoke).toHaveBeenNthCalledWith(1, "get_board", {
			appId: "app-1",
			boardId: "board-1",
			version: [3, 0, 0],
		});
		expect(mocks.invoke).toHaveBeenNthCalledWith(2, "get_flowscript", {
			appId: "app-1",
			boardId: "board-1",
			version: [3, 0, 0],
			anchors: true,
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("reads an exact hosted FlowScript revision directly", async () => {
		mocks.fetcher.mockResolvedValueOnce({ flowscript: "event entry() {}\n" });
		const state = new BoardState(hostedBackend() as never);

		await expect(
			state.getFlowScriptAuthoritative("app-1", "board-1", [4, 2, 0], false),
		).resolves.toBe("event entry() {}\n");
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app-1/board/board-1/flowscript?version=4_2_0&anchors=false",
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("propagates a hosted Event inventory failure without a local fallback", async () => {
		const failure = new Error("authority unavailable");
		mocks.fetcher.mockRejectedValueOnce(failure);
		const state = new EventState(hostedBackend() as never);

		await expect(state.getEventsAuthoritative("app-1")).rejects.toBe(failure);
		expect(mocks.fetcher.mock.calls[0][1]).toBe("apps/app-1/events");
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("reads an exact hosted Event revision without caching it locally", async () => {
		const event = { id: "event-1", active: false };
		const backend = hostedBackend();
		mocks.fetcher.mockResolvedValueOnce(event);
		const state = new EventState(backend as never);

		await expect(
			state.getEventAuthoritative("app-1", "event-1", [5, 0, 1]),
		).resolves.toBe(event);
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"apps/app-1/events/event-1?version=5_0_1",
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(backend.queryClient.setQueryData).not.toHaveBeenCalled();
	});

	test("keeps local-only Event reads native and rejects missing versions", async () => {
		const failure = new Error("version missing");
		mocks.invoke.mockRejectedValueOnce(failure);
		const state = new EventState(localBackend() as never);

		await expect(
			state.getEventAuthoritative("app-1", "event-1", [6, 0, 0]),
		).rejects.toBe(failure);
		expect(mocks.invoke).toHaveBeenCalledOnce();
		expect(mocks.invoke).toHaveBeenCalledWith("get_event", {
			appId: "app-1",
			eventId: "event-1",
			version: [6, 0, 0],
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("requires authenticated authority before a hosted read", async () => {
		const backend = hostedBackend(false);

		await expect(
			new BoardState(backend as never).getBoardAuthoritative(
				"app-1",
				"board-1",
			),
		).rejects.toThrow("authenticated hub session");
		await expect(
			new EventState(backend as never).getEventsAuthoritative("app-1"),
		).rejects.toThrow("authenticated hub session");
		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});
