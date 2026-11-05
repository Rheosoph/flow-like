// @vitest-environment happy-dom
import type {
	InvalidateOptions,
	InvalidateQueryFilters,
	Query,
} from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type {
	RealtimeClientOptions,
	RealtimeListener,
	RealtimeUpdate,
} from "../../../packages/ui/lib/realtime/types";

const harness = vi.hoisted(() => ({
	auth: {
		isAuthenticated: true,
		user: {
			access_token: "token-a",
			profile: { sub: "user-a", iss: "issuer" },
		},
	},
	backend: {
		userState: {
			getNotifications: function getNotifications() {},
			listNotifications: function listNotifications() {},
		},
		teamState: { getInvites: function getInvites() {} },
		registryState: { listMyInvitations: function listMyInvitations() {} },
	},
	queryClient: {
		invalidateQueries: vi.fn<
			(
				filters: InvalidateQueryFilters,
				options?: InvalidateOptions,
			) => Promise<void>
		>(async () => {}),
	},
	prepare: vi.fn(),
	getClient: vi.fn(),
	incomingCalls: vi.fn(() => null),
}));

vi.mock("@flow-like/flow-like-ui/components/incoming-call-provider", () => ({
	IncomingCallProvider: harness.incomingCalls,
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => harness.backend,
}));
vi.mock("@flow-like/flow-like-ui/lib/api-url", () => ({
	getApiOrigin: () => "https://hub.test",
}));
vi.mock("@flow-like/flow-like-ui/lib/realtime/runtime-client", () => ({
	getRealtimeClient: harness.getClient,
}));
vi.mock("@tanstack/react-query", () => ({
	useQueryClient: () => harness.queryClient,
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => harness.auth }));
vi.mock("../lib/session-request", () => ({
	prepareSessionRequest: harness.prepare,
}));

import { WebRealtimeBridge } from "./web-realtime-bridge";

let root: Root;
let host: HTMLDivElement;
const connections: ReturnType<typeof fakeClient>[] = [];
function fakeClient() {
	let listener: RealtimeListener | undefined;
	return {
		setSession: vi.fn(),
		setSubscriptions: vi.fn(),
		setActivity: vi.fn(),
		close: vi.fn(),
		subscribe: vi.fn((callback: RealtimeListener) => {
			listener = callback;
			return () => {
				listener = undefined;
			};
		}),
		emit: (update: RealtimeUpdate) => listener?.(update),
	};
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	harness.auth = {
		isAuthenticated: true,
		user: {
			access_token: "token-a",
			profile: { sub: "user-a", iss: "issuer" },
		},
	};
	harness.prepare.mockReset().mockImplementation(async () => ({
		headers: new Headers({
			Authorization: `Bearer ${harness.auth.user.access_token}`,
		}),
		onUnauthorized: vi.fn(async () => ({ Authorization: "Bearer renewed" })),
	}));
	harness.getClient.mockReset().mockImplementation(async () => {
		const client = fakeClient();
		connections.push(client);
		return client;
	});
	harness.queryClient.invalidateQueries.mockClear();
	harness.incomingCalls.mockClear();
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	connections.length = 0;
	vi.useRealTimers();
});
async function render(enabled = true) {
	await act(async () => root.render(<WebRealtimeBridge enabled={enabled} />));
}

const changed: RealtimeUpdate = {
	type: "snapshot",
	snapshot: {
		protocol_version: 1,
		notifications: {
			revision: "n1",
			overview: { invites_count: 0, notifications_count: 1, unread_count: 1 },
		},
		runs: [],
		errors: [],
	},
};

describe("authenticated web realtime bridge", () => {
	test("keeps notification and invitation queries current", async () => {
		await render();
		expect(connections[0].setSubscriptions).toHaveBeenCalledWith({
			notifications: true,
			backgroundNotifications: true,
			runs: [],
			conversations: [],
		});
		await connections[0].emit(changed);
		expect(
			harness.queryClient.invalidateQueries.mock.calls.flatMap(
				([options]) => options.queryKey?.[0] ?? [],
			),
		).toEqual([
			"getNotifications",
			"listNotifications",
			"getInvites",
			"listMyInvitations",
		]);
	});

	test("refreshes scoped home notification widgets without touching other home queries", async () => {
		await render();
		await connections[0].emit(changed);
		const homeInvalidation =
			harness.queryClient.invalidateQueries.mock.calls.find(
				([filters]) => filters.predicate,
			);
		expect(homeInvalidation?.[1]).toEqual({ throwOnError: true });
		const matches = (queryKey: readonly unknown[]) =>
			homeInvalidation?.[0].predicate?.({ queryKey } as Query);
		expect(
			matches([
				"home",
				"hub",
				"issuer",
				"user",
				"notifications",
				true,
				"ALL",
				8,
			]),
		).toBe(true);
		expect(matches(["home", "hub", "issuer", "user", "activity"])).toBe(false);
		expect(matches(["notifications"])).toBe(false);
		expect(matches(["other", "hub", "issuer", "user", "notifications"])).toBe(
			false,
		);
	});

	test("waits for refetch and leaves a failed snapshot retryable", async () => {
		await render();
		let reject!: (error: Error) => void;
		harness.queryClient.invalidateQueries.mockReturnValueOnce(
			new Promise<void>((_resolve, rejectPromise) => {
				reject = rejectPromise;
			}),
		);
		const acceptance = connections[0].emit(changed);
		let accepted = false;
		void acceptance?.then(
			() => {
				accepted = true;
			},
			() => {},
		);
		await Promise.resolve();
		expect(accepted).toBe(false);
		expect(harness.queryClient.invalidateQueries).toHaveBeenCalledWith(
			{ queryKey: ["getNotifications"] },
			{ throwOnError: true },
		);
		reject(new Error("notification refresh failed"));
		await expect(acceptance).rejects.toThrow("notification refresh failed");
		await expect(connections[0].emit(changed)).resolves.toBeUndefined();
		expect(harness.queryClient.invalidateQueries).toHaveBeenCalledTimes(12);
	});

	test("rotates credentials without opening another connection", async () => {
		await render();
		harness.auth = {
			...harness.auth,
			user: { ...harness.auth.user, access_token: "token-b" },
		};
		await render();
		expect(harness.getClient).toHaveBeenCalledTimes(1);
		expect(connections[0].setSession).toHaveBeenLastCalledWith(
			expect.objectContaining({ headers: { Authorization: "Bearer token-b" } }),
		);
	});

	test("adopts credentials rotated while the worker was starting", async () => {
		let finish!: (client: ReturnType<typeof fakeClient>) => void;
		harness.getClient.mockReturnValueOnce(
			new Promise((resolve) => {
				finish = resolve;
			}),
		);
		await render();
		harness.auth = {
			...harness.auth,
			user: { ...harness.auth.user, access_token: "token-b" },
		};
		await render();
		const client = fakeClient();
		await act(async () => finish(client));
		expect(client.setSession).toHaveBeenLastCalledWith(
			expect.objectContaining({ headers: { Authorization: "Bearer token-b" } }),
		);
		expect(client.setSubscriptions).toHaveBeenCalledWith({
			notifications: true,
			backgroundNotifications: true,
			runs: [],
			conversations: [],
		});
	});

	test("closes the previous account and rejects its queued callback", async () => {
		await render();
		const stale = connections[0].subscribe.mock.calls[0][0];
		harness.auth = {
			...harness.auth,
			user: {
				access_token: "token-b",
				profile: { sub: "user-b", iss: "issuer" },
			},
		};
		await render();
		expect(connections[0].close).toHaveBeenCalledTimes(1);
		stale(changed);
		expect(harness.queryClient.invalidateQueries).not.toHaveBeenCalled();
		expect(harness.getClient).toHaveBeenCalledTimes(2);
	});

	test("stays off until the authenticated shell is ready", async () => {
		await render(false);
		expect(harness.getClient).not.toHaveBeenCalled();
		expect(harness.incomingCalls).toHaveBeenLastCalledWith(
			expect.objectContaining({ enabled: false }),
			undefined,
		);
		harness.auth.isAuthenticated = false;
		await render(true);
		expect(harness.getClient).not.toHaveBeenCalled();
		expect(harness.incomingCalls).toHaveBeenLastCalledWith(
			expect.objectContaining({ enabled: false }),
			undefined,
		);
	});

	test("scopes incoming calls to the current authenticated account", async () => {
		await render();
		expect(harness.incomingCalls).toHaveBeenLastCalledWith(
			{
				enabled: true,
				scope: JSON.stringify(["https://hub.test", "issuer", "user-a"]),
				userId: "user-a",
			},
			undefined,
		);
		harness.auth.user.profile.sub = "user-b";
		await render();
		expect(harness.incomingCalls).toHaveBeenLastCalledWith(
			{
				enabled: true,
				scope: JSON.stringify(["https://hub.test", "issuer", "user-b"]),
				userId: "user-b",
			},
			undefined,
		);
	});

	test("retries a transient startup failure", async () => {
		vi.useFakeTimers();
		harness.prepare.mockRejectedValueOnce(new Error("network unavailable"));
		await render();
		expect(harness.getClient).not.toHaveBeenCalled();
		await act(async () => {
			await vi.advanceTimersByTimeAsync(1_000);
		});
		expect(harness.getClient).toHaveBeenCalledTimes(1);
	});

	test("closes a partially started client before retrying session preparation", async () => {
		vi.useFakeTimers();
		harness.prepare
			.mockResolvedValueOnce({
				headers: new Headers({ Authorization: "Bearer token-a" }),
			})
			.mockRejectedValueOnce(new Error("session temporarily unavailable"));
		await render();
		expect(connections).toHaveLength(1);
		expect(connections[0].close).toHaveBeenCalledTimes(1);
		expect(connections[0].setSubscriptions).not.toHaveBeenCalled();
		await act(async () => {
			await vi.advanceTimersByTimeAsync(1_000);
		});
		expect(connections).toHaveLength(2);
		expect(connections[1].setSubscriptions).toHaveBeenCalledWith({
			notifications: true,
			backgroundNotifications: true,
			runs: [],
			conversations: [],
		});
		expect(connections[0].close).toHaveBeenCalledTimes(1);
	});

	test("does not renew an old account after logout", async () => {
		await render();
		const options = harness.getClient.mock.calls[0][1] as RealtimeClientOptions;
		harness.auth.isAuthenticated = false;
		await render(false);
		expect(await options.onUnauthorized?.("token-a")).toBeUndefined();
	});
});
