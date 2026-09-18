import type { IEvent } from "@flow-like/flow-like-ui";
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
		active: true,
		event_type: "cron",
		execution_mode: "Local",
		board_id: "",
		config: [],
		...overrides,
	} as IEvent;
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
