import type { IEvent } from "@flow-like/flow-like-ui";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	streamFetcher: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
	// The real Channel reaches into the Tauri window globals, which no test has.
	Channel: class {
		onmessage: ((events: unknown) => void) | undefined;
	},
}));

vi.mock("../../../lib/api", () => ({
	fetcher: mocks.fetcher,
	streamFetcher: mocks.streamFetcher,
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

import { EventState } from "../event-state";

const APP = "app-1";
const EVENT = "event-1";
const BOARD = "board-1";

const remoteEvent = (): IEvent =>
	({
		id: EVENT,
		board_id: BOARD,
		execution_mode: "Remote",
		event_type: "chat",
		name: "Chat",
		node_id: "node-1",
		priority: 0,
		active: true,
		variables: {},
		config: [],
		event_version: [0, 0, 1],
		description: "",
		created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	}) as unknown as IEvent;

function fakeBackend(overrides: { localOnly?: boolean } = {}) {
	return {
		// Unknown visibility reads as offline, which is exactly why nothing here
		// may use it to rule out the server.
		isOffline: vi.fn().mockResolvedValue(true),
		isLocalOnly: vi.fn().mockResolvedValue(overrides.localOnly ?? false),
		profile: { id: "profile-1", hub: "hub-1" },
		auth: { user: { access_token: "token-1", profile: { sub: "user-1" } } },
		queryClient: { setQueryData: vi.fn(), invalidateQueries: vi.fn() },
		backgroundTaskHandler: vi.fn(),
		boardState: {
			// The board this device cannot read — either never downloaded, or the
			// caller has no permission to see the flow. Reading it is the failure
			// every path here has to avoid.
			getBoard: vi.fn().mockRejectedValue(new Error("board not found")),
			ensureAppPackagesInstalledForExecution: vi.fn(),
		},
	};
}

beforeEach(() => {
	mocks.invoke.mockReset();
	mocks.fetcher.mockReset();
	mocks.streamFetcher.mockReset();
});

/**
 * An app whose execution is pinned to the server exists precisely so its data
 * never reaches the device, so the board file is not there to read. Every step
 * that runs before execution has to settle on "server" from the event record
 * alone — a device that reaches for the board only fails on the way to a run
 * that was always going to happen elsewhere.
 */
describe("an event pinned to Remote execution", () => {
	test("executes on the server without reading a local board", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return remoteEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(remoteEvent());
		mocks.streamFetcher.mockResolvedValue(undefined);
		const backend = fakeBackend();
		const state = new EventState(backend as never);

		await state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never);

		expect(mocks.streamFetcher).toHaveBeenCalledTimes(1);
		expect(mocks.streamFetcher.mock.calls[0][1]).toBe(
			`apps/${APP}/events/${EVENT}/invoke`,
		);
		expect(backend.boardState.getBoard).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"execute_event",
			expect.anything(),
		);
	});

	test("preflight reports remote-only even when the API is unreachable", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return remoteEvent();
			if (command === "get_board") throw new Error("board not found");
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockRejectedValue(new Error("offline"));
		const state = new EventState(fakeBackend() as never);

		const prerun = await state.prerunEvent(APP, EVENT);

		expect(prerun.can_execute_locally).toBe(false);
		expect(prerun.execution_mode).toBe("Remote");
		expect(prerun.event_execution_mode).toBe("Remote");
		expect(prerun.board_id).toBe(BOARD);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"get_board",
			expect.anything(),
		);
	});

	test("chat OAuth preflight settles without reaching for a board", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend();
		const state = new EventState(backend as never);

		const result = await state.checkEventOAuth(APP, remoteEvent());

		expect(result).toEqual({ missingProviders: [] });
		expect(backend.boardState.getBoard).not.toHaveBeenCalled();
	});

	test("preflight prefers the server's answer when it is reachable", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return remoteEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue({
			board_id: BOARD,
			runtime_variables: [{ id: "var-1", name: "Key", secret: false }],
			oauth_requirements: [],
			requires_local_execution: false,
			execution_mode: "Remote",
			event_execution_mode: "Remote",
			can_execute_locally: false,
			has_wasm_nodes: false,
			wasm_package_ids: [],
			wasm_package_permissions: {},
		});
		const state = new EventState(fakeBackend() as never);

		const prerun = await state.prerunEvent(APP, EVENT);

		expect(prerun.runtime_variables).toHaveLength(1);
		expect(prerun.can_execute_locally).toBe(false);
	});
});

/**
 * A published app's users hold ExecuteEvents without ReadBoards — they may run
 * the flow but never see it. Their board reads fail on permission rather than
 * on a missing file, and the run still has to happen.
 */
describe("a caller who may run the event but not read its board", () => {
	const localEvent = () =>
		({ ...remoteEvent(), execution_mode: "Local" }) as unknown as IEvent;

	test("runs on the server instead of failing on the board", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(localEvent());
		mocks.streamFetcher.mockResolvedValue(undefined);
		const state = new EventState(fakeBackend() as never);

		await state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never);

		expect(mocks.streamFetcher).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"execute_event",
			expect.anything(),
		);
	});

	test("preflight falls through to the server's answer", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localEvent();
			if (command === "get_board") throw new Error("forbidden");
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue({
			board_id: BOARD,
			runtime_variables: [],
			oauth_requirements: [],
			requires_local_execution: false,
			execution_mode: "Hybrid",
			event_execution_mode: "Local",
			can_execute_locally: false,
			has_wasm_nodes: false,
			wasm_package_ids: [],
			wasm_package_permissions: {},
		});
		const state = new EventState(fakeBackend() as never);

		const prerun = await state.prerunEvent(APP, EVENT);

		expect(prerun.can_execute_locally).toBe(false);
	});

	test("chat OAuth preflight yields to the server instead of failing", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend();
		const state = new EventState(backend as never);

		const result = await state.checkEventOAuth(APP, localEvent());

		expect(result).toEqual({ missingProviders: [] });
		expect(backend.boardState.getBoard).toHaveBeenCalledTimes(1);
	});

	test("a local-only app still surfaces the board failure", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		const state = new EventState(fakeBackend({ localOnly: true }) as never);

		await expect(
			state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never),
		).rejects.toThrow("board not found");
		expect(mocks.streamFetcher).not.toHaveBeenCalled();
	});
});

/** A Local event on a device that holds the board keeps its local preflight. */
describe("an event that runs on this device", () => {
	test("preflight still reads the local board", async () => {
		const localEvent = { ...remoteEvent(), execution_mode: "Local" };
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localEvent;
			if (command === "get_board")
				return {
					id: BOARD,
					variables: {},
					nodes: {},
					layers: {},
					execution_mode: "Hybrid",
				};
			throw new Error(`unexpected invoke: ${command}`);
		});
		const state = new EventState(fakeBackend() as never);

		const prerun = await state.prerunEvent(APP, EVENT);

		expect(prerun.event_execution_mode).toBe("Local");
		expect(mocks.invoke).toHaveBeenCalledWith(
			"get_board",
			expect.objectContaining({ boardId: BOARD }),
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});
});
