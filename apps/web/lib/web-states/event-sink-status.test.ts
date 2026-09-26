import type { IEvent } from "@flow-like/flow-like-ui";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiGet: vi.fn() }));

vi.mock("@flow-like/flow-like-ui", () => ({}));
vi.mock("./api-utils", () => ({ apiGet: mocks.apiGet }));
vi.mock("../oauth-db", () => ({ oauthConsentStore: {}, oauthTokenStore: {} }));
vi.mock("../oauth-service", () => ({}));
vi.mock("sonner", () => ({ toast: vi.fn() }));

import { WebEventState } from "./event-state";
import { WebSinkState } from "./sink-state";

const backend = { auth: { isAuthenticated: true } } as never;

describe("web event trigger status", () => {
	beforeEach(() => vi.resetAllMocks());

	test.each([WebEventState, WebSinkState])(
		"reads status through the existing singular sink endpoint (%s)",
		async (State) => {
			mocks.apiGet.mockResolvedValue({ active: true });
			await expect(
				new State(backend).isEventSinkActive("event-1"),
			).resolves.toBe(true);
			expect(mocks.apiGet).toHaveBeenCalledWith("sink/event-1", {
				isAuthenticated: true,
			});
		},
	);

	test.each([WebEventState, WebSinkState])(
		"distinguishes missing sinks from failed reads (%s)",
		async (State) => {
			const failure = Object.assign(new Error("hub unavailable"), {
				status: 503,
			});
			mocks.apiGet
				.mockRejectedValueOnce({ status: 404, code: "NOT_FOUND" })
				.mockRejectedValueOnce(failure);
			const state = new State(backend);
			await expect(state.isEventSinkActive("event-1")).resolves.toBe(false);
			await expect(state.isEventSinkActive("event-1")).rejects.toBe(failure);
		},
	);

	test.each(["rest", "mcp"])(
		"uses the %s event enabled flag without requiring a worker sink",
		async (type) => {
			const state = new WebEventState(backend);
			const event = { active: true, event_type: type } as IEvent;
			await expect(
				state.isEventSinkActive("event-1", { appId: "app-1", event }),
			).resolves.toBe(true);
			await expect(
				state.isEventSinkActive("event-1", {
					appId: "app-1",
					event: { ...event, active: false },
				}),
			).resolves.toBe(false);
			expect(mocks.apiGet).not.toHaveBeenCalled();
		},
	);
});
