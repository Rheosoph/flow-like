import {
	type IEvent,
	resetPageContractDrift,
	subscribeToPageContractDrift,
} from "@flow-like/flow-like-ui";
import { startRunTrace } from "@flow-like/flow-like-ui/lib/run-timing";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { ApiResponseError } from "../../../lib/api-error";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	streamFetcher: vi.fn(),
	dispatchPaymentRequest: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui/components/payments/payment-events", () => ({
	dispatchPaymentRequest: mocks.dispatchPaymentRequest,
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

vi.mock("../../../lib/oauth-db", () => ({
	oauthConsentStore: {
		getConsentedProviderIds: vi.fn().mockResolvedValue(new Set()),
	},
	oauthTokenStore: {},
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

// The remote-known Event markers persist through localStorage, which the node
// test environment does not provide.
if (typeof globalThis.localStorage === "undefined") {
	const store = new Map<string, string>();
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			getItem: (key: string) => store.get(key) ?? null,
			setItem: (key: string, value: string) => {
				store.set(key, String(value));
			},
			removeItem: (key: string) => {
				store.delete(key);
			},
			clear: () => {
				store.clear();
			},
		},
	});
}

import { EventState, mergeLocalAndRemoteEvents } from "../event-state";

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
		prepareExecutionAuth: vi.fn().mockResolvedValue("https://hub-1"),
		executionSessionId: "native-window-session",
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
	mocks.dispatchPaymentRequest.mockReset();
	localStorage.clear();
	resetPageContractDrift();
});

async function tracedStepNames(run: () => Promise<unknown>) {
	const info = vi.spyOn(console, "info").mockImplementation(() => {});
	try {
		const trace = startRunTrace("test");
		await run().catch(() => undefined);
		return trace.finish()?.steps.map((step) => step.name) ?? [];
	} finally {
		info.mockRestore();
	}
}

describe("event snapshot freshness", () => {
	test("offline upserts preserve reserved Event IDs on creation and retry", async () => {
		const backend = fakeBackend({ localOnly: true });
		const state = new EventState(backend as never);
		const event = {
			...remoteEvent(),
			id: "fp_event_reserved",
			board_id: "",
			active: false,
		};
		const saved = new Map<string, IEvent>();
		mocks.invoke.mockImplementation(async (command, args) => {
			expect(command).toBe("upsert_event");
			const id = args.enforceId ? args.event.id : `replacement-${saved.size}`;
			const persisted = { ...args.event, id };
			saved.set(id, persisted);
			return persisted;
		});
		expect((await state.upsertEvent(APP, event)).id).toBe(event.id);
		expect((await state.upsertEvent(APP, event)).id).toBe(event.id);
		expect(saved.size).toBe(1);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("times the local mirror write after a remote read", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event" || command === "upsert_event")
				return remoteEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(remoteEvent());
		const backend = fakeBackend();
		backend.isOffline.mockResolvedValue(false);
		const state = new EventState(backend as never);

		expect(await tracedStepNames(() => state.getEvent(APP, EVENT))).toEqual([
			"get_event.local",
			"get_event.is_offline",
			"get_event.remote",
			"get_event.upsert_local",
		]);
		expect(mocks.invoke).toHaveBeenCalledWith(
			"upsert_event",
			expect.objectContaining({ appId: APP, enforceId: true }),
		);
	});

	test("keeps a strictly newer local Hybrid event", () => {
		const local = {
			...remoteEvent(),
			execution_mode: "Hybrid",
			updated_at: { secs_since_epoch: 3, nanos_since_epoch: 0 },
		} as unknown as IEvent;
		const remote = {
			...remoteEvent(),
			updated_at: { secs_since_epoch: 2, nanos_since_epoch: 0 },
		};

		expect(mergeLocalAndRemoteEvents([local], [remote])).toEqual([local]);
	});

	test("accepts a newer remote event and drops a removed Remote-only cache", () => {
		const local = {
			...remoteEvent(),
			execution_mode: "Hybrid",
			updated_at: { secs_since_epoch: 2, nanos_since_epoch: 0 },
		} as unknown as IEvent;
		const remote = {
			...remoteEvent(),
			updated_at: { secs_since_epoch: 3, nanos_since_epoch: 0 },
		};
		const removedRemoteOnly = {
			...remoteEvent(),
			id: "removed-remote-event",
		};

		expect(
			mergeLocalAndRemoteEvents([local, removedRemoteOnly], [remote]),
		).toEqual([remote]);
	});

	test("retains a device-local event missing from the remote mirror", () => {
		const local = {
			...remoteEvent(),
			id: "device-local-event",
			execution_mode: "Local",
		} as unknown as IEvent;

		expect(mergeLocalAndRemoteEvents([local], [])).toEqual([local]);
	});
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

	test("times a stream that fails before the run id arrives", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return remoteEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(remoteEvent());
		mocks.streamFetcher.mockRejectedValue(new Error("gateway timeout"));
		const state = new EventState(fakeBackend() as never);

		const steps = await tracedStepNames(() =>
			state.executeEvent(
				APP,
				EVENT,
				{ id: EVENT, payload: {} } as never,
				undefined,
				() => {},
			),
		);

		expect(steps).toContain("remote.until_error");
		expect(steps).not.toContain("remote.until_run_id");
	});

	test("cancelling a server run stops it on the API and aborts its stream here", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return remoteEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockImplementation(
			async (_profile: unknown, _path: string, options?: RequestInit) =>
				options?.method === "DELETE" ? { cancelled: true } : remoteEvent(),
		);
		let emit: (event: unknown) => void = () => {};
		let signal: AbortSignal | undefined;
		// Like the real stream, an aborted signal errors the body being read.
		mocks.streamFetcher.mockImplementation(
			(
				_profile: unknown,
				_path: string,
				options: RequestInit,
				_auth: unknown,
				onMessage: (event: unknown) => void,
			) =>
				new Promise<void>((_, reject) => {
					emit = onMessage;
					signal = options.signal ?? undefined;
					signal?.addEventListener("abort", () => reject("Request cancelled"));
				}),
		);
		const backend = fakeBackend();
		const state = new EventState(backend as never);
		const cb = vi.fn();
		const onEventId = vi.fn();
		const running = state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			onEventId,
			cb,
		);
		await vi.waitFor(() => expect(mocks.streamFetcher).toHaveBeenCalled());
		emit({ event_type: "run_initiated", payload: { run_id: "server/run" } });
		expect(onEventId).toHaveBeenCalledWith("server/run");
		expect(signal?.aborted).toBe(false);

		await state.cancelExecution("server/run");
		expect(signal?.aborted).toBe(true);
		expect(mocks.fetcher).toHaveBeenCalledWith(
			backend.profile,
			"execution/run/server%2Frun",
			{ method: "DELETE" },
			backend.auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"cancel_execution",
			expect.anything(),
		);
		const delivered = cb.mock.calls.length;
		emit({ event_type: "a2ui", payload: { type: "showScreen" } });
		expect(cb).toHaveBeenCalledTimes(delivered);

		// The aborted stream ends the run the way a server-closed one does.
		await expect(running).resolves.toBeUndefined();
		mocks.invoke.mockResolvedValue(undefined);
		await state.cancelExecution("server/run");
		expect(mocks.invoke).toHaveBeenCalledWith("cancel_execution", {
			runId: "server/run",
		});
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

	test("hands a server run's payment request to the payment prompt", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return remoteEvent();
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(remoteEvent());
		const request = {
			event_type: "payment_request",
			payload: { id: "pay_1", appId: APP, runId: "run-1" },
		};
		mocks.streamFetcher.mockImplementation(
			async (
				_profile: unknown,
				_path: string,
				_options: RequestInit,
				_auth: unknown,
				onMessage: (event: unknown) => void,
			) => onMessage(request),
		);
		const state = new EventState(fakeBackend() as never);

		await state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never);

		expect(mocks.dispatchPaymentRequest).toHaveBeenCalledWith(request);
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
	function runnableLocalEvent() {
		const event = { ...remoteEvent(), execution_mode: "Local" };
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return event;
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue({
			id: BOARD,
			variables: {},
			nodes: {},
			layers: {},
			execution_mode: "Hybrid",
		});
		return { backend, event, state: new EventState(backend as never) };
	}

	test("an offline project forwards its window identity and the token current after auth sync", async () => {
		const { backend, state } = runnableLocalEvent();
		backend.prepareExecutionAuth.mockImplementation(async () => {
			backend.auth.user.access_token = "rotated-token";
			return "https://hub-1";
		});

		await state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never);

		expect(backend.prepareExecutionAuth).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({
				executionHub: "https://hub-1",
				executionSessionId: "native-window-session",
				token: "rotated-token",
			}),
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("signed-out offline execution does not require the login bridge", async () => {
		const { backend, state } = runnableLocalEvent();
		backend.auth = { user: null } as never;
		backend.prepareExecutionAuth.mockRejectedValue(new Error("Sign in"));

		await state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never);

		expect(backend.prepareExecutionAuth).not.toHaveBeenCalled();
		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({
				executionHub: undefined,
				executionSessionId: undefined,
				token: undefined,
			}),
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("a failed login bridge still permits offline work without borrowing another session", async () => {
		const { backend, state } = runnableLocalEvent();
		backend.prepareExecutionAuth.mockRejectedValue(
			new Error("Bridge unavailable"),
		);

		await state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never);

		expect(backend.prepareExecutionAuth).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({
				executionHub: undefined,
				executionSessionId: undefined,
			}),
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("a failed login bridge blocks online execution before native dispatch", async () => {
		const { backend, event, state } = runnableLocalEvent();
		backend.isOffline.mockResolvedValue(false);
		backend.isLocalOnly.mockResolvedValue(false);
		mocks.fetcher.mockResolvedValue(event);
		backend.prepareExecutionAuth.mockRejectedValue(
			new Error("Bridge unavailable"),
		);

		await expect(
			state.executeEvent(APP, EVENT, { id: EVENT, payload: {} } as never),
		).rejects.toThrow("Bridge unavailable");

		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"execute_event",
			expect.anything(),
		);
	});

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

describe("a registry-backed local Page action", () => {
	test("prefers the local Hybrid path when the exact Page contract is present", async () => {
		const localPageEvent = {
			...remoteEvent(),
			execution_mode: "Local",
			default_page_id: "page-1",
		};
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-current" };
			}
			if (command === "get_event") return localPageEvent;
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue({
			board_id: BOARD,
			runtime_variables: [],
			oauth_requirements: [],
			requires_local_execution: false,
			execution_mode: "Hybrid",
			event_execution_mode: "Local",
			can_execute_locally: true,
			has_wasm_nodes: false,
			wasm_package_ids: [],
			wasm_package_permissions: {},
			manifest_revision: "per2-current",
		});
		const backend = fakeBackend();
		backend.profile = { id: "profile-1" } as never;
		backend.boardState.getBoard.mockResolvedValue({
			id: BOARD,
			variables: {},
			nodes: {},
			layers: {},
			execution_mode: "Hybrid",
		});
		const state = new EventState(backend as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "pa1_static",
				manifestRevision: "per2-current",
			},
		);

		expect(mocks.streamFetcher).not.toHaveBeenCalled();
		expect(mocks.invoke).toHaveBeenCalledWith("get_local_page_bootstrap", {
			appId: APP,
			eventId: EVENT,
		});
		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({
				appId: APP,
				eventId: EVENT,
				pageTrigger: {
					kind: "action",
					action_id: "pa1_static",
					manifest_revision: "per2-current",
				},
			}),
		);
	});

	test("falls back remotely before execution when the exact Page contract is not local", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_local_page_bootstrap") {
				throw new Error("versioned Page is not local");
			}
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue({
			board_id: BOARD,
			runtime_variables: [],
			oauth_requirements: [],
			requires_local_execution: false,
			execution_mode: "Hybrid",
			event_execution_mode: "Local",
			can_execute_locally: true,
			has_wasm_nodes: false,
			wasm_package_ids: [],
			wasm_package_permissions: {},
			manifest_revision: "per2-current",
		});
		mocks.streamFetcher.mockResolvedValue(undefined);
		const state = new EventState(fakeBackend() as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "pa1_static",
				manifestRevision: "per2-current",
			},
		);

		expect(mocks.streamFetcher).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"execute_event",
			expect.anything(),
		);
	});

	test("does not revive a remote-known Event the hub reports as deleted", async () => {
		const localPageEvent = {
			...remoteEvent(),
			execution_mode: "Local",
			default_page_id: "page-1",
		};
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent;
			if (command === "upsert_event") return undefined;
			if (command === "delete_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend();
		backend.isOffline.mockResolvedValue(false);
		const state = new EventState(backend as never);

		// The server acknowledged the Event once, so a later 404 is a revocation.
		mocks.fetcher.mockResolvedValueOnce(localPageEvent);
		await state.getEvent(APP, EVENT);

		mocks.fetcher.mockRejectedValue(
			new ApiResponseError({
				status: 404,
				code: "NOT_FOUND",
				message: "Event not found",
				path: `apps/${APP}/events/${EVENT}`,
			}),
		);

		await expect(state.getEvent(APP, EVENT)).rejects.toMatchObject({
			status: 404,
		});
		expect(mocks.invoke).toHaveBeenCalledWith("delete_event", {
			appId: APP,
			eventId: EVENT,
		});
	});

	test("keeps a never-uploaded local Event the server has not acknowledged", async () => {
		const localPageEvent = {
			...remoteEvent(),
			execution_mode: "Local",
			default_page_id: "page-1",
		};
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent;
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockRejectedValue(
			new ApiResponseError({
				status: 404,
				code: "NOT_FOUND",
				message: "Event not found",
				path: `apps/${APP}/events/${EVENT}`,
			}),
		);
		const backend = fakeBackend();
		backend.isOffline.mockResolvedValue(false);
		const state = new EventState(backend as never);

		await expect(state.getEvent(APP, EVENT)).resolves.toEqual(localPageEvent);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"delete_event",
			expect.anything(),
		);
	});

	test("uses local prerun without exposing its id to the API", async () => {
		const localPageEvent = {
			...remoteEvent(),
			execution_mode: "Local",
			default_page_id: "page-1",
		};
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent;
			if (command === "get_board") {
				return {
					id: BOARD,
					variables: {},
					nodes: {},
					layers: {},
					execution_mode: "Hybrid",
				};
			}
			throw new Error(`unexpected invoke: ${command}`);
		});
		const state = new EventState(fakeBackend() as never);

		const prerun = await state.prerunEvent(APP, EVENT, undefined, {
			kind: "action",
			actionId: "lda1_native-grant",
			manifestRevision: "per2-current",
		});

		expect(prerun.event_execution_mode).toBe("Local");
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("cannot enter the remote execution path", async () => {
		const state = new EventState(fakeBackend() as never);

		await expect(
			state.executeEventRemote(
				APP,
				EVENT,
				{ id: EVENT, payload: {} } as never,
				undefined,
				undefined,
				undefined,
				{
					kind: "action",
					actionId: "lda1_native-grant",
					capabilityJwt: "untrusted-server-token",
					manifestRevision: "per2-current",
				},
			),
		).rejects.toThrow("cannot be sent to the server");
		expect(mocks.streamFetcher).not.toHaveBeenCalled();
	});
});

/**
 * The pre-run gate asks whether this device holds a Page contract for the
 * Event at all — not whether it holds the *exact* revision the rendered Page
 * carries. That revision hashes the whole Board, so any unrelated edit
 * supersedes it while the Page keeps rendering; demanding equality here sent a
 * runnable action to the server, and on a local-only app failed it outright.
 * The native command re-resolves the trigger against the current contract.
 */
describe("the pre-run Page contract gate", () => {
	const hybridPrerun = () => ({
		board_id: BOARD,
		runtime_variables: [],
		oauth_requirements: [],
		requires_local_execution: false,
		execution_mode: "Hybrid",
		event_execution_mode: "Local",
		can_execute_locally: true,
		has_wasm_nodes: false,
		wasm_package_ids: [],
		wasm_package_permissions: {},
		manifest_revision: "per2-current",
	});

	const localPageEvent = () => ({
		...remoteEvent(),
		execution_mode: "Local",
		default_page_id: "page-1",
	});

	const emptyHybridBoard = () => ({
		id: BOARD,
		variables: {},
		nodes: {},
		layers: {},
		execution_mode: "Hybrid",
	});

	test("still runs locally when a board edit superseded the rendered revision", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-stale" };
			}
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(hybridPrerun());
		const backend = fakeBackend();
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "pa1_static",
				manifestRevision: "per2-current",
			},
		);

		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({ appId: APP, eventId: EVENT }),
		);
		expect(mocks.streamFetcher).not.toHaveBeenCalled();
	});

	test("routes remotely when the trigger carries no revision", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-current" };
			}
			throw new Error(`unexpected invoke: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(hybridPrerun());
		mocks.streamFetcher.mockResolvedValue(undefined);
		const state = new EventState(fakeBackend() as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{ kind: "action", actionId: "pa1_static" },
		);

		expect(mocks.streamFetcher).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"execute_event",
			expect.anything(),
		);
	});

	test("a local-only app with a superseded revision runs instead of failing", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-stale" };
			}
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "pa1_static",
				manifestRevision: "per2-current",
			},
		);

		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({ appId: APP, eventId: EVENT }),
		);
		expect(mocks.streamFetcher).not.toHaveBeenCalled();
	});

	test("throws an actionable error instead of a doomed hub call when unreachable", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			// No Page contract on this device at all — not merely a superseded one.
			if (command === "get_local_page_bootstrap") {
				throw new Error("No active Event was found");
			}
			throw new Error(`unexpected invoke: ${command}`);
		});
		const state = new EventState(fakeBackend({ localOnly: true }) as never);

		await expect(
			state.executeEvent(
				APP,
				EVENT,
				{ id: EVENT, payload: {} } as never,
				undefined,
				undefined,
				undefined,
				undefined,
				{
					kind: "action",
					actionId: "pa1_static",
					manifestRevision: "per2-current",
				},
			),
		).rejects.toThrow(
			"This device holds no Page contract for this Event and the server is unreachable; reload the Page",
		);
		expect(mocks.streamFetcher).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("re-stamps the native invoke with the device's current revision", async () => {
		// The rendered Page carries the revision it was built with; any unrelated
		// Board edit supersedes it. The device just told us the current one, so the
		// click runs with that rather than being refused or shipped to a hub.
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-device" };
			}
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "pa1_static",
				manifestRevision: "per2-rendered",
			},
		);

		const call = mocks.invoke.mock.calls.find(
			([command]) => command === "execute_event",
		);
		expect(call?.[1]?.pageTrigger).toEqual({
			kind: "action",
			action_id: "pa1_static",
			manifest_revision: "per2-device",
		});
	});

	test("never re-stamps a local dynamic grant", async () => {
		// An lda1_ grant is minted against one exact revision and the native gate
		// still compares it. Substituting would resurrect a revoked capability.
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "lda1_native-grant",
				manifestRevision: "per2-rendered",
			},
		);

		const call = mocks.invoke.mock.calls.find(
			([command]) => command === "execute_event",
		);
		expect(call?.[1]?.pageTrigger).toEqual({
			kind: "action",
			action_id: "lda1_native-grant",
			manifest_revision: "per2-rendered",
		});
	});

	test("publishes a drift signal when the native command refuses the contract", async () => {
		// Tauri rejects with the SERIALIZED error object, so this also proves the
		// classifier reads `{ error }` rather than only `message`.
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-device" };
			}
			if (command === "execute_event") {
				throw { error: "The Page action is stale or invalid" };
			}
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		const seen: string[] = [];
		const off = subscribeToPageContractDrift((detail) =>
			seen.push(detail.reason),
		);

		await expect(
			state.executeEvent(
				APP,
				EVENT,
				{ id: EVENT, payload: {} } as never,
				undefined,
				undefined,
				undefined,
				undefined,
				{
					kind: "action",
					actionId: "pa1_static",
					manifestRevision: "per2-rendered",
				},
			),
		).rejects.toBeDefined();
		off();

		expect(seen).toEqual(["stale_action"]);
	});

	test("times a native dispatch that fails before run_initiated", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-device" };
			}
			if (command === "execute_event") {
				throw { error: "Page local execution is not authorized" };
			}
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		const steps = await tracedStepNames(() =>
			state.executeEvent(
				APP,
				EVENT,
				{ id: EVENT, payload: {} } as never,
				undefined,
				() => {},
				undefined,
				undefined,
				{
					kind: "action",
					actionId: "pa1_static",
					manifestRevision: "per2-device",
				},
			),
		);

		expect(steps).toContain("execute_event.until_error");
		expect(steps).not.toContain("execute_event.until_run_initiated");
	});

	test("a successful run publishes nothing", async () => {
		// A completed run has already rewritten the surface through its own A2UI
		// messages. Refetching on top of that would re-run onLoad over live
		// content and duplicate or discard what the click just produced.
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "get_local_page_bootstrap") {
				return { executionRevision: "per2-device" };
			}
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend({ localOnly: true });
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		const seen: string[] = [];
		const off = subscribeToPageContractDrift((detail) =>
			seen.push(detail.reason),
		);
		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "pa1_static",
				manifestRevision: "per2-rendered",
			},
		);
		off();

		expect(seen).toEqual([]);
	});

	test("a local dynamic Page action skips the contract gate", async () => {
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command === "get_event") return localPageEvent();
			if (command === "get_board") return emptyHybridBoard();
			if (command === "execute_event") return undefined;
			throw new Error(`unexpected invoke: ${command}`);
		});
		const backend = fakeBackend();
		backend.profile = { id: "profile-1" } as never;
		backend.boardState.getBoard.mockResolvedValue(emptyHybridBoard());
		const state = new EventState(backend as never);

		await state.executeEvent(
			APP,
			EVENT,
			{ id: EVENT, payload: {} } as never,
			undefined,
			undefined,
			undefined,
			undefined,
			{
				kind: "action",
				actionId: "lda1_native-grant",
				manifestRevision: "per2-current",
			},
		);

		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"get_local_page_bootstrap",
			expect.anything(),
		);
		expect(mocks.invoke).toHaveBeenCalledWith(
			"execute_event",
			expect.objectContaining({ appId: APP, eventId: EVENT }),
		);
		expect(mocks.streamFetcher).not.toHaveBeenCalled();
	});
});
