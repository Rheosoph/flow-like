import { type IEvent, IEventExecutionMode } from "@flow-like/flow-like-ui";
import { withDeviceEventSource } from "@flow-like/flow-like-ui/lib/event-source";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	IEventExecutionMode: { Local: "Local", Remote: "Remote" },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../api", () => ({ fetcher: mocks.fetcher, streamFetcher: vi.fn() }));
vi.mock("../oauth-db", () => ({ oauthConsentStore: {}, oauthTokenStore: {} }));
vi.mock("../oauth-service", () => ({ oauthService: {} }));
vi.mock("../../components/rpa", () => ({}));
vi.mock("sonner", () => ({ toast: vi.fn() }));

import { EventState } from "../../components/tauri-provider/event-state";

function event(eventType = "cron", target?: string): IEvent {
	return {
		id: "event-1",
		name: "Scheduled report",
		description: "",
		active: true,
		event_type: eventType,
		execution_mode: IEventExecutionMode.Local,
		board_id: "board-1",
		node_id: "node-1",
		config: Array.from(
			new TextEncoder().encode(JSON.stringify({ sink_execution: target })),
		),
		event_version: [0, 0, 0],
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		priority: 0,
		variables: {},
	};
}

function remote(eventType = "cron"): IEvent {
	return { ...event(eventType), execution_mode: IEventExecutionMode.Remote };
}

function backend(localOnly = false) {
	return {
		profile: { id: "profile-1", hub: "hub.example" },
		auth: { isAuthenticated: true, user: { access_token: "token" } },
		isLocalOnly: vi.fn().mockResolvedValue(localOnly),
	};
}

function context(value: IEvent) {
	return { appId: "app-1", event: value };
}

function onDevice(state: unknown) {
	mocks.invoke.mockImplementation((command: string) => {
		if (command === "local_trigger_status") return Promise.resolve(state);
		throw new Error(`Unexpected command: ${command}`);
	});
}

function invokedCommands() {
	return mocks.invoke.mock.calls.map(([command]) => command);
}

describe("desktop event trigger status", () => {
	beforeEach(() => vi.resetAllMocks());

	test.each(["rest", "mcp", "cron"])(
		"reports a deployed %s definition inactive at its source",
		async (type) => {
			const state = new EventState(backend() as never);
			await expect(
				state.isEventSinkActive(
					"event-1",
					context(withDeviceEventSource(event(type))),
				),
			).resolves.toBe(false);
			expect(mocks.invoke).not.toHaveBeenCalled();
			expect(mocks.fetcher).not.toHaveBeenCalled();
		},
	);

	test.each(["cron", "api", "http"])(
		"reads a remote %s trigger from its hosted sink",
		async (type) => {
			mocks.fetcher.mockResolvedValue({ active: true });
			const stateBackend = backend();
			const state = new EventState(stateBackend as never);

			await expect(
				state.isEventSinkActive("event-1", context(event(type, "remote"))),
			).resolves.toBe(true);
			expect(mocks.invoke).not.toHaveBeenCalled();
			expect(mocks.fetcher).toHaveBeenCalledWith(
				stateBackend.profile,
				"sink/event-1",
				{ method: "GET" },
				stateBackend.auth,
			);
		},
	);

	test.each(["LOCAL", undefined])(
		"reads a %s trigger from the hosted sink when its workflow runs remotely",
		async (target) => {
			onDevice("none");
			mocks.fetcher.mockResolvedValue({ active: true });
			const value: IEvent = {
				...event("cron", target),
				execution_mode: IEventExecutionMode.Remote,
			};
			const stateBackend = backend();
			const state = new EventState(stateBackend as never);

			await expect(
				state.isEventSinkActive("event-1", context(value)),
			).resolves.toBe(true);
			expect(mocks.fetcher).toHaveBeenCalledWith(
				stateBackend.profile,
				"sink/event-1",
				{ method: "GET" },
				stateBackend.auth,
			);
			expect(invokedCommands()).toEqual(["local_trigger_status"]);
		},
	);

	test("reads a Remote location region from this device, which forwards its crossings", async () => {
		onDevice("ready");
		const value = remote("geolocation");
		const state = new EventState(backend() as never);

		await expect(
			state.getEventSinkStatus("event-1", context(value)),
		).resolves.toEqual({ active: true });
		expect(mocks.invoke).toHaveBeenCalledWith("local_trigger_status", {
			appId: "app-1",
			event: value,
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("reports a forwarded region without a token as needing one", async () => {
		onDevice("needs_token");
		const state = new EventState(backend() as never);

		await expect(
			state.getEventSinkStatus("event-1", context(remote("geolocation"))),
		).resolves.toEqual({ active: false, needs_token: true });
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test.each([
		["ready", { active: true }],
		["unregistered", { active: false }],
		["needs_token", { active: false, needs_token: true }],
		[
			{ failed: { reason: "bot login failed" } },
			{ active: false, failed_reason: "bot login failed" },
		],
	])("reads a trigger on this computer in state %o", async (local, status) => {
		onDevice(local);
		const state = new EventState(backend() as never);

		await expect(
			state.getEventSinkStatus("event-1", context(event("cron"))),
		).resolves.toEqual(status);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("without context only asks whether the registration can fire", async () => {
		mocks.invoke.mockResolvedValue(true);
		const state = new EventState(backend() as never);

		await expect(state.getEventSinkStatus("event-1")).resolves.toEqual({
			active: true,
		});
		expect(mocks.invoke).toHaveBeenCalledWith("is_event_sink_active", {
			eventId: "event-1",
		});
	});

	test("reports a remote workflow's trigger inactive in a local-only app", async () => {
		const state = new EventState(backend(true) as never);

		await expect(
			state.isEventSinkActive("event-1", context(remote("discord"))),
		).resolves.toBe(false);
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test.each(["rest", "mcp"])(
		"uses the enabled flag for hosted %s even with a Local workflow",
		async (type) => {
			const state = new EventState(backend() as never);
			await expect(
				state.isEventSinkActive("event-1", context(event(type))),
			).resolves.toBe(true);
			expect(mocks.invoke).not.toHaveBeenCalled();
			expect(mocks.fetcher).not.toHaveBeenCalled();
		},
	);

	test("reports a hybrid trigger active if either runtime is active", async () => {
		onDevice("unregistered");
		mocks.fetcher.mockResolvedValue({ active: true });
		const state = new EventState(backend() as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).resolves.toBe(true);
	});

	test("keeps a hybrid trigger active when one status read fails", async () => {
		onDevice("ready");
		mocks.fetcher.mockRejectedValue(new Error("hub unavailable"));
		const state = new EventState(backend() as never);
		await expect(
			state.getEventSinkStatus("event-1", context(event("cron", "HYBRID"))),
		).resolves.toEqual({ active: true });
	});

	test("preserves an unknown hybrid status if neither runtime proves active", async () => {
		const failure = new Error("hub unavailable");
		onDevice("unregistered");
		mocks.fetcher.mockRejectedValue(failure);
		const state = new EventState(backend() as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).rejects.toBe(failure);
	});

	test("checks only the device for a hybrid trigger in a local-only app", async () => {
		onDevice("unregistered");
		const state = new EventState(backend(true) as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).resolves.toBe(false);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("distinguishes a missing hosted sink from a failed status read", async () => {
		const failure = Object.assign(new Error("forbidden"), { status: 403 });
		mocks.fetcher
			.mockRejectedValueOnce({ status: 404, code: "NOT_FOUND" })
			.mockRejectedValueOnce(failure);
		const state = new EventState(backend() as never);
		const value = context(event("cron", "REMOTE"));
		await expect(state.isEventSinkActive("event-1", value)).resolves.toBe(
			false,
		);
		await expect(state.isEventSinkActive("event-1", value)).rejects.toBe(
			failure,
		);
	});

	test("reports whom a remote workflow's hosted sink runs as, and nobody when that user left", async () => {
		onDevice("none");
		const value = context(remote("cron"));
		mocks.fetcher
			.mockResolvedValueOnce({ active: true, run_as_user_id: "user-1" })
			.mockResolvedValueOnce({ active: false, run_as_user_id: null });
		const state = new EventState(backend() as never);
		await expect(state.getEventSinkStatus("event-1", value)).resolves.toEqual({
			active: true,
			run_as_user_id: "user-1",
		});
		await expect(state.getEventSinkStatus("event-1", value)).resolves.toEqual({
			active: false,
			run_as_user_id: null,
		});
	});

	test("passes on a hosted sink that still runs on a saved token", async () => {
		onDevice("none");
		mocks.fetcher.mockResolvedValue({
			active: true,
			run_as_user_id: null,
			legacy_token: true,
		});
		const state = new EventState(backend() as never);
		await expect(
			state.getEventSinkStatus("event-1", context(remote("cron"))),
		).resolves.toEqual({
			active: true,
			run_as_user_id: null,
			legacy_token: true,
		});
	});

	test("a hybrid trigger keeps this computer's missing token next to the hub's run-as user", async () => {
		onDevice("needs_token");
		mocks.fetcher.mockResolvedValue({
			active: true,
			run_as_user_id: "user-1",
			legacy_token: false,
		});
		const state = new EventState(backend() as never);
		await expect(
			state.getEventSinkStatus("event-1", context(event("cron", "HYBRID"))),
		).resolves.toEqual({
			active: true,
			needs_token: true,
			run_as_user_id: "user-1",
			legacy_token: false,
		});
	});

	test("a hybrid trigger names no run-as user for a paused hub sink behind an active local trigger", async () => {
		onDevice("ready");
		mocks.fetcher.mockResolvedValue({ active: false, run_as_user_id: null });
		const state = new EventState(backend() as never);
		const status = await state.getEventSinkStatus(
			"event-1",
			context(event("cron", "HYBRID")),
		);
		expect(status).toEqual({ active: true });
		expect(status).not.toHaveProperty("run_as_user_id");
	});

	test("a hybrid trigger that runs nowhere tells why this computer's does not", async () => {
		onDevice({ failed: { reason: "bot login failed" } });
		mocks.fetcher.mockResolvedValue({ active: false, run_as_user_id: null });
		const state = new EventState(backend() as never);
		await expect(
			state.getEventSinkStatus("event-1", context(event("cron", "HYBRID"))),
		).resolves.toEqual({
			active: false,
			failed_reason: "bot login failed",
			run_as_user_id: null,
		});
	});

	test("keeps disabled events inactive without probing a runtime", async () => {
		const state = new EventState(backend() as never);
		await expect(
			state.isEventSinkActive(
				"event-1",
				context({ ...event(), active: false }),
			),
		).resolves.toBe(false);
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});
});
