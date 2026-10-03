import type { IEvent } from "@flow-like/flow-like-ui";
import { withDeviceEventSource } from "@flow-like/flow-like-ui/lib/event-source";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiPut: vi.fn(), apiGet: vi.fn() }));

vi.mock("@flow-like/flow-like-ui", () => ({}));
vi.mock("./api-utils", () => ({ apiPut: mocks.apiPut, apiGet: mocks.apiGet }));
vi.mock("../oauth-db", () => ({ oauthConsentStore: {}, oauthTokenStore: {} }));
vi.mock("../oauth-service", () => ({}));
vi.mock("sonner", () => ({ toast: vi.fn() }));

import { WebEventState } from "./event-state";

const backend = {
	auth: { isAuthenticated: true },
	profile: { id: "profile-1" },
} as never;
const event: IEvent = {
	id: "event-1",
	name: "Device endpoint",
	description: "",
	active: true,
	event_type: "rest",
	board_id: "board-1",
	node_id: "node-1",
	config: [],
	event_version: [0, 0, 0],
	created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	priority: 0,
	variables: {},
};

describe("device event creation", () => {
	beforeEach(() => {
		vi.resetAllMocks();
		mocks.apiPut.mockResolvedValue(event);
		mocks.apiGet.mockResolvedValue({ device_event_creation: true });
	});

	test("saves a deployable definition with source registration disabled", async () => {
		await expect(
			new WebEventState(backend).upsertEvent(
				"app-1",
				event,
				undefined,
				undefined,
				undefined,
				{ source: "device" },
			),
		).resolves.toEqual(event);

		expect(mocks.apiPut).toHaveBeenCalledWith(
			"apps/app-1/events/event-1",
			expect.objectContaining({
				event: withDeviceEventSource(event),
				register_source: false,
				profile_id: "profile-1",
			}),
			{ isAuthenticated: true },
		);
	});

	test("keeps the source disabled after a normal edit of a device event", async () => {
		const updated = withDeviceEventSource({
			...event,
			name: "Updated endpoint",
		});
		await new WebEventState(backend).upsertEvent("app-1", updated);
		expect(mocks.apiPut.mock.calls[0]?.[1]).toMatchObject({
			event: updated,
			register_source: false,
		});
	});

	test("preserves ordinary source registration when no device target is selected", async () => {
		await new WebEventState(backend).upsertEvent("app-1", event);
		expect(mocks.apiPut.mock.calls[0]?.[1]).not.toHaveProperty(
			"register_source",
		);
	});

	test("refuses older hubs before creating an event that could activate its source", async () => {
		mocks.apiGet.mockResolvedValue({});
		await expect(
			new WebEventState(backend).upsertEvent(
				"app-1",
				event,
				undefined,
				undefined,
				undefined,
				{ source: "device" },
			),
		).rejects.toThrow("Update the hub");
		expect(mocks.apiPut).not.toHaveBeenCalled();
	});
});
