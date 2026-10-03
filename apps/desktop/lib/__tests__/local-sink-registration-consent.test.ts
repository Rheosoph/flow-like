import { type IEvent, IEventExecutionMode } from "@flow-like/flow-like-ui";
import { withDeviceEventSource } from "@flow-like/flow-like-ui/lib/event-source";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	consent: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	IEventExecutionMode: { Local: "Local", Remote: "Remote" },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../api", () => ({ fetcher: mocks.fetcher, streamFetcher: vi.fn() }));
vi.mock("../oauth-db", () => ({ oauthConsentStore: {}, oauthTokenStore: {} }));
vi.mock("../oauth-service", () => ({ oauthService: {} }));
vi.mock("../../components/rpa", () => ({}));
vi.mock("../../components/local-sink/local-sink-consent", () => ({
	requestLocalSinkConsent: mocks.consent,
}));
vi.mock("sonner", () => ({ toast: vi.fn() }));

import { EventState } from "../../components/tauri-provider/event-state";

function event(overrides: Partial<IEvent> = {}): IEvent {
	return {
		id: "event-1",
		name: "Nightly report",
		description: "",
		active: true,
		event_type: "cron",
		execution_mode: IEventExecutionMode.Local,
		board_id: "",
		node_id: "node-1",
		config: [],
		event_version: [0, 0, 0],
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		priority: 0,
		variables: {},
		...overrides,
	};
}

function offlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(true),
		boardState: { getBoard: vi.fn() },
	};
}

function onlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(false),
		boardState: { getBoard: vi.fn() },
		profile: { id: "profile-1" },
		auth: { user: { access_token: "token" } },
		queryClient: { setQueryData: vi.fn() },
	};
}

function backendPlans(plan: "none" | "existing" | "new") {
	mocks.invoke.mockImplementation((command: string) => {
		if (command === "local_sink_registration_plan") {
			return Promise.resolve(plan);
		}
		if (command === "upsert_event") return Promise.resolve(event());
		if (command === "restore_event") {
			return Promise.resolve({ plan: { restored: event(), issues: [] } });
		}
		throw new Error(`Unexpected command: ${command}`);
	});
}

function calls(command: string) {
	return mocks.invoke.mock.calls
		.filter(([name]) => name === command)
		.map(([, args]) => args as Record<string, unknown>);
}

describe("local trigger consent when saving an event", () => {
	beforeEach(() => {
		vi.resetAllMocks();
		backendPlans("new");
	});

	test("registers a new trigger once the user allows it", async () => {
		mocks.consent.mockResolvedValue("allow");
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(calls("local_sink_registration_plan")[0]).toMatchObject({
			appId: "app-1",
		});
		expect(mocks.consent).toHaveBeenCalledWith({
			appId: "app-1",
			eventId: "event-1",
			eventName: "Nightly report",
			eventType: "cron",
		});
		expect(calls("upsert_event")[0]?.registerSink).toBe("register");
	});

	test("saves the event without a trigger when the user declines", async () => {
		mocks.consent.mockResolvedValue("decline");
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(calls("upsert_event")[0]?.registerSink).toBe("skip");
	});

	test("creates a device event without registering a source trigger or requesting local permissions", async () => {
		const backend = offlineBackend();
		const state = new EventState(backend as never);
		const target = event({ board_id: "board-1" });

		await state.upsertEvent("app-1", target, undefined, undefined, undefined, {
			source: "device",
		});

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(backend.boardState.getBoard).not.toHaveBeenCalled();
		expect(calls("local_sink_registration_plan")).toHaveLength(0);
		expect(calls("upsert_event")).toEqual([
			expect.objectContaining({
				event: withDeviceEventSource(target),
				enforceId: true,
				registerSink: "skip",
			}),
		]);
		expect(target.active).toBe(true);
	});

	test("disables both hub and local source triggers when creating an online device event", async () => {
		const state = new EventState(onlineBackend() as never);
		const target = event();
		mocks.fetcher
			.mockResolvedValueOnce({ device_event_creation: true })
			.mockResolvedValueOnce(withDeviceEventSource(target));

		await state.upsertEvent("app-1", target, undefined, undefined, undefined, {
			source: "device",
		});

		const request = mocks.fetcher.mock.calls[1]?.[2] as { body: string };
		expect(JSON.parse(request.body)).toMatchObject({
			register_source: false,
			event: { id: target.id, active: true },
		});
		expect(calls("upsert_event")[0]).toMatchObject({
			event: withDeviceEventSource(target),
			registerSink: "skip",
		});
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("refuses an older hub before writing either source copy", async () => {
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockResolvedValue({});

		await expect(
			state.upsertEvent("app-1", event(), undefined, undefined, undefined, {
				source: "device",
			}),
		).rejects.toThrow("Update the hub");

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("editing a device event keeps source triggers disabled without creation options", async () => {
		const state = new EventState(offlineBackend() as never);
		const target = withDeviceEventSource(event({ name: "Updated schedule" }));

		await state.upsertEvent("app-1", target);

		expect(calls("upsert_event")[0]).toMatchObject({
			event: target,
			registerSink: "skip",
		});
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("reports a failed removal of the source trigger before deployment can start", async () => {
		const state = new EventState(offlineBackend() as never);
		mocks.invoke.mockRejectedValue(new Error("Source trigger removal failed"));

		await expect(
			state.upsertEvent("app-1", event(), undefined, undefined, undefined, {
				source: "device",
			}),
		).rejects.toThrow("Source trigger removal failed");
	});

	test("abandons the save when the dialog is dismissed", async () => {
		mocks.consent.mockResolvedValue("dismiss");
		const state = new EventState(offlineBackend() as never);

		await expect(state.upsertEvent("app-1", event())).rejects.toMatchObject({
			isLocalSinkConsentDismissed: true,
		});
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("refreshes an approved trigger without asking again", async () => {
		backendPlans("existing");
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]?.registerSink).toBe("register");
	});

	test("does not ask when nothing would register", async () => {
		backendPlans("none");
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event({ event_type: "chat" }));

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]?.registerSink).toBe("keep");
	});

	test("leaves an inactive event to the backend without asking", async () => {
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event({ active: false }));

		expect(calls("local_sink_registration_plan")).toHaveLength(0);
		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]?.registerSink).toBe("keep");
	});

	test("asks before the server write and passes the answer to the local mirror", async () => {
		mocks.consent.mockResolvedValue("decline");
		const saved = event({ name: "Saved on the server" });
		mocks.fetcher.mockImplementation(async () => {
			expect(mocks.consent).toHaveBeenCalled();
			return saved;
		});
		const state = new EventState(onlineBackend() as never);

		await expect(state.upsertEvent("app-1", event())).resolves.toEqual(saved);

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(calls("upsert_event")[0]).toMatchObject({
			event: saved,
			registerSink: "skip",
		});
	});

	test("a non-dry restore goes through the same approval", async () => {
		mocks.consent.mockResolvedValue("allow");
		const state = new EventState(offlineBackend() as never);

		await state.restoreEvent("app-1", "event-1", [0, 0, 1], {
			dryRun: false,
		});

		const restores = calls("restore_event");
		expect(restores.map((args) => args.dryRun)).toEqual([true, false]);
		expect(restores[1]?.registerSink).toBe("register");
		expect(mocks.consent).toHaveBeenCalledTimes(1);
	});

	test("a dry-run restore never asks", async () => {
		const state = new EventState(offlineBackend() as never);

		await state.restoreEvent("app-1", "event-1", [0, 0, 1]);

		expect(calls("restore_event")).toHaveLength(1);
		expect(calls("local_sink_registration_plan")).toHaveLength(0);
	});
});
