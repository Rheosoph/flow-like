import type { IEvent, IRouteMapping } from "@flow-like/flow-like-ui";
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

import {
	EventState,
	mergeLocalAndRemoteEvents,
} from "../../components/tauri-provider/event-state";
import {
	RouteState,
	mergeLocalAndRemoteRoutes,
} from "../../components/tauri-provider/route-state";

function event(
	id: string,
	priority: number,
	executionMode?: "Local" | "Remote",
): IEvent {
	return {
		id,
		name: `${id}-local`,
		priority,
		execution_mode: executionMode,
	} as IEvent;
}

function updatedAt(eventValue: IEvent, seconds: number): IEvent {
	return {
		...eventValue,
		updated_at: {
			secs_since_epoch: seconds,
			nanos_since_epoch: 0,
		},
	};
}

function onlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(false),
		profile: { id: "profile-1" },
		auth: { user: { access_token: "token" } },
		queryClient: { setQueryData: vi.fn() },
		backgroundTaskHandler: vi.fn(),
	};
}

beforeEach(() => {
	mocks.invoke.mockReset();
	mocks.fetcher.mockReset();
});

describe("desktop event snapshot merging", () => {
	test("keeps local events when the remote DB mirror is incomplete", () => {
		const local = [event("local", 2, "Local"), event("legacy", 1)];

		expect(mergeLocalAndRemoteEvents(local, [])).toEqual([local[1], local[0]]);
	});

	test("lets remote records refresh matching IDs without retaining stale remote-only events", () => {
		const localVersion = event("shared", 5, "Local");
		const staleRemote = event("removed-remote", 1, "Remote");
		const remoteVersion = {
			...event("shared", 2, "Remote"),
			name: "shared-from-server",
		};
		const newRemote = event("new-remote", 3, "Remote");

		expect(
			mergeLocalAndRemoteEvents(
				[localVersion, staleRemote],
				[remoteVersion, newRemote],
			),
		).toEqual([remoteVersion, newRemote]);
	});

	test("keeps a strictly newer local match but lets remote win equal timestamps", () => {
		const newerLocal = updatedAt(event("newer-local", 1, "Local"), 20);
		const staleRemote = updatedAt(
			{
				...event("newer-local", 1, "Local"),
				name: "stale-server-copy",
			},
			10,
		);
		const equalLocal = updatedAt(event("equal", 2, "Local"), 10);
		const equalRemote = updatedAt(
			{
				...event("equal", 2, "Local"),
				name: "equal-server-copy",
			},
			10,
		);

		expect(
			mergeLocalAndRemoteEvents(
				[newerLocal, equalLocal],
				[staleRemote, equalRemote],
			),
		).toEqual([newerLocal, equalRemote]);
	});

	test("does not overwrite a newer local event while syncing a stale remote copy", async () => {
		const newerLocal = updatedAt(event("shared", 1, "Local"), 20);
		const staleRemote = updatedAt(
			{
				...event("shared", 1, "Local"),
				name: "stale-server-copy",
			},
			10,
		);
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "get_events") return Promise.resolve([newerLocal]);
			if (command === "upsert_event") return Promise.resolve(undefined);
			throw new Error(`Unexpected command: ${command}`);
		});
		mocks.fetcher.mockResolvedValue([staleRemote]);
		const backend = onlineBackend();
		const state = new EventState(backend as never);

		await expect(state.getEvents("app-1", true)).resolves.toEqual([newerLocal]);
		expect(mocks.invoke).not.toHaveBeenCalledWith(
			"upsert_event",
			expect.anything(),
		);
	});

	test("uses a fetched event even when persisting its local cache entry fails", async () => {
		const remoteEvent = event("remote", 0, "Remote");
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "get_events") return Promise.resolve([]);
			if (command === "upsert_event") {
				return Promise.reject(new Error("local board is not available yet"));
			}
			throw new Error(`Unexpected command: ${command}`);
		});
		mocks.fetcher.mockResolvedValue([remoteEvent]);
		const backend = onlineBackend();
		const state = new EventState(backend as never);

		await expect(state.getEvents("app-1", true)).resolves.toEqual([
			remoteEvent,
		]);
		expect(backend.queryClient.setQueryData).toHaveBeenCalledWith(
			["getEvents", "app-1", true],
			[remoteEvent],
		);
		expect(warn).toHaveBeenCalledWith(
			"[EventSync] Failed to persist remote event remote locally:",
			expect.any(Error),
		);
		warn.mockRestore();
	});
});

describe("desktop route snapshot merging", () => {
	test("keeps local-only paths and lets the server replace path conflicts", () => {
		const local: IRouteMapping[] = [
			{ path: "/", eventId: "local-home" },
			{ path: "/local", eventId: "local-event" },
		];
		const remote: IRouteMapping[] = [
			{ path: "/", eventId: "remote-home" },
			{ path: "/remote", eventId: "remote-event" },
		];

		expect(mergeLocalAndRemoteRoutes(local, remote)).toEqual([
			{ path: "/", eventId: "remote-home" },
			{ path: "/local", eventId: "local-event" },
			{ path: "/remote", eventId: "remote-event" },
		]);
	});

	test("returns merged paths from a forced online refresh", async () => {
		const local = [{ path: "/", eventId: "local-event" }];
		const remote = [{ path: "/remote", eventId: "remote-event" }];
		mocks.invoke.mockImplementation((command: string) => {
			if (command === "get_app_routes") return Promise.resolve(local);
			if (command === "set_app_route") return Promise.resolve(undefined);
			throw new Error(`Unexpected command: ${command}`);
		});
		mocks.fetcher.mockResolvedValue(remote);
		const backend = onlineBackend();
		const state = new RouteState(backend as never);

		await expect(state.getRoutes("app-1", true)).resolves.toEqual([
			...local,
			...remote,
		]);
		expect(backend.queryClient.setQueryData).toHaveBeenCalledWith(
			["getRoutes", "app-1", true],
			[...local, ...remote],
		);
	});
});
