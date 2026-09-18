import type { IEvent } from "@flow-like/flow-like-ui";
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
		active: true,
		event_type: eventType,
		execution_mode: "Local",
		config: Array.from(
			new TextEncoder().encode(JSON.stringify({ sink_execution: target })),
		),
	} as IEvent;
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

describe("desktop event trigger status", () => {
	beforeEach(() => vi.resetAllMocks());

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
			mocks.fetcher.mockResolvedValue({ active: true });
			const value = {
				...event("cron", target),
				execution_mode: "Remote",
			} as IEvent;
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
			expect(mocks.invoke).not.toHaveBeenCalled();
		},
	);

	test("reports a remote workflow's trigger inactive in a local-only app", async () => {
		const value = { ...event("discord"), execution_mode: "Remote" } as IEvent;
		const state = new EventState(backend(true) as never);

		await expect(
			state.isEventSinkActive("event-1", context(value)),
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
		mocks.invoke.mockResolvedValue(false);
		mocks.fetcher.mockResolvedValue({ active: true });
		const state = new EventState(backend() as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).resolves.toBe(true);
	});

	test("keeps a hybrid trigger active when one status read fails", async () => {
		mocks.invoke.mockResolvedValue(true);
		mocks.fetcher.mockRejectedValue(new Error("hub unavailable"));
		const state = new EventState(backend() as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).resolves.toBe(true);
	});

	test("preserves an unknown hybrid status if neither runtime proves active", async () => {
		const failure = new Error("hub unavailable");
		mocks.invoke.mockResolvedValue(false);
		mocks.fetcher.mockRejectedValue(failure);
		const state = new EventState(backend() as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).rejects.toBe(failure);
	});

	test("checks only the device for a hybrid trigger in a local-only app", async () => {
		mocks.invoke.mockResolvedValue(false);
		const state = new EventState(backend(true) as never);
		await expect(
			state.isEventSinkActive("event-1", context(event("cron", "HYBRID"))),
		).resolves.toBe(false);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("distinguishes a missing hosted sink from a failed status read", async () => {
		const failure = Object.assign(new Error("forbidden"), { status: 403 });
		mocks.fetcher
			.mockRejectedValueOnce({ status: 404 })
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
