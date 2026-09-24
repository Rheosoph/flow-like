import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { afterEach, describe, expect, test, vi } from "vitest";

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: vi
		.fn()
		.mockImplementation(async (command: string) =>
			command === "execution_open_auth_session" ? "native-session" : undefined,
		),
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

import { TauriBackend } from "../../tauri-provider";
import { subscribeWidgetQueryRefresh } from "../widget-query-refresh";

const profile = { id: "profile-1", hub: "hub-1" };
const auth = {
	isLoading: false,
	isAuthenticated: true,
	user: { access_token: "token", profile: { sub: "user-1" } },
};

const cleanups: (() => void)[] = [];

afterEach(() => {
	for (const cleanup of cleanups.splice(0)) cleanup();
	vi.useRealTimers();
});

function cachedWidgets() {
	const queryClient = new QueryClient({
		defaultOptions: {
			queries: {
				staleTime: Number.POSITIVE_INFINITY,
				refetchOnWindowFocus: false,
				refetchOnReconnect: false,
			},
		},
	});
	cleanups.push(() => queryClient.clear());
	const queries = [
		["getWidgets", "app-1"],
		["getWidget", "app-1", "widget-1"],
	].map((queryKey) => {
		queryClient.setQueryData(queryKey, "local widget cache");
		const queryFn = vi.fn().mockResolvedValue("remote widgets");
		const observer = new QueryObserver(queryClient, { queryKey, queryFn });
		cleanups.push(observer.subscribe(() => {}));
		return { queryKey, queryFn };
	});
	return { queryClient, queries };
}

async function expectRemoteWidgets({
	queryClient,
	queries,
}: ReturnType<typeof cachedWidgets>) {
	await vi.waitFor(() => {
		for (const { queryKey, queryFn } of queries) {
			expect(queryFn).toHaveBeenCalledTimes(1);
			expect(queryClient.getQueryData(queryKey)).toBe("remote widgets");
		}
	});
}

describe("native widget cache after session bootstrap", () => {
	test("refreshes mounted widgets when the profile arrives after auth", async () => {
		const cached = cachedWidgets();
		const backend = new TauriBackend(
			() => undefined,
			cached.queryClient,
			auth as never,
		);

		backend.pushQueryClient(cached.queryClient);
		for (const { queryFn } of cached.queries) {
			expect(queryFn).not.toHaveBeenCalled();
		}

		backend.pushProfile(profile as never);

		await expectRemoteWidgets(cached);
	});

	test("refreshes mounted widgets when auth finishes after the profile", async () => {
		const cached = cachedWidgets();
		const backend = new TauriBackend(() => undefined, cached.queryClient, {
			...auth,
			isLoading: true,
		} as never);

		backend.pushProfile(profile as never);
		for (const { queryFn } of cached.queries) {
			expect(queryFn).not.toHaveBeenCalled();
		}

		backend.pushAuthContext(auth as never);

		await expectRemoteWidgets(cached);
	});

	test("refreshes widget queries when the query client attaches last", async () => {
		const cached = cachedWidgets();
		const backend = new TauriBackend(
			() => undefined,
			undefined,
			auth as never,
			profile as never,
		);

		backend.pushQueryClient(cached.queryClient);

		await expectRemoteWidgets(cached);
	});

	test.each([
		["getWidgets", "app-1"],
		["getWidget", "app-1", "widget-1"],
	])(
		"restarts an unfinished local %s read once auth is ready",
		async (...queryKey) => {
			const queryClient = new QueryClient({
				defaultOptions: { queries: { staleTime: Number.POSITIVE_INFINITY } },
			});
			cleanups.push(() => queryClient.clear());
			let finishLocalRead: (value: string) => void = () => {};
			const queryFn = vi
				.fn()
				.mockImplementationOnce(
					() =>
						new Promise<string>((resolve) => {
							finishLocalRead = resolve;
						}),
				)
				.mockResolvedValue("remote widgets");
			const observer = new QueryObserver(queryClient, { queryKey, queryFn });
			cleanups.push(observer.subscribe(() => {}));
			expect(queryFn).toHaveBeenCalledTimes(1);
			const backend = new TauriBackend(
				() => undefined,
				queryClient,
				auth as never,
			);

			backend.pushProfile(profile as never);
			finishLocalRead("local widget cache");

			await vi.waitFor(() => {
				expect(queryFn).toHaveBeenCalledTimes(2);
				expect(queryClient.getQueryData(queryKey)).toBe("remote widgets");
			});
		},
	);
});

describe("native widget cache after returning to an app", () => {
	function mountedWidgetRefresh() {
		const cached = cachedWidgets();
		const backend = new TauriBackend(
			() => undefined,
			cached.queryClient,
			auth as never,
			profile as never,
		);
		const windowTarget = new EventTarget();
		const documentTarget = Object.assign(new EventTarget(), {
			visibilityState: "visible" as DocumentVisibilityState,
		});
		const unsubscribe = subscribeWidgetQueryRefresh(
			() => backend.refreshWidgetQueries(),
			windowTarget,
			documentTarget,
		);
		cleanups.push(unsubscribe);
		return { ...cached, backend, windowTarget, documentTarget, unsubscribe };
	}

	test.each(["focus", "online", "visibilitychange"])(
		"refreshes mounted widgets on %s even when the profile has not changed",
		async (event) => {
			const mounted = mountedWidgetRefresh();
			const target =
				event === "visibilitychange"
					? mounted.documentTarget
					: mounted.windowTarget;
			target.dispatchEvent(new Event(event));

			await expectRemoteWidgets(mounted);
			expect(mounted.backend.profile).toBe(profile);
		},
	);

	test("waits until the document is visible after reconnecting in the background", async () => {
		const mounted = mountedWidgetRefresh();
		mounted.documentTarget.visibilityState = "hidden";
		mounted.windowTarget.dispatchEvent(new Event("online"));
		mounted.documentTarget.dispatchEvent(new Event("visibilitychange"));
		for (const { queryFn } of mounted.queries) {
			expect(queryFn).not.toHaveBeenCalled();
		}

		mounted.documentTarget.visibilityState = "visible";
		mounted.documentTarget.dispatchEvent(new Event("visibilitychange"));

		await expectRemoteWidgets(mounted);
	});

	test("coalesces resume events without refetching unrelated app data", async () => {
		const mounted = mountedWidgetRefresh();
		const queryKey = ["getBoard", "app-1", "board-1"];
		mounted.queryClient.setQueryData(queryKey, "cached board");
		const queryFn = vi.fn().mockResolvedValue("remote board");
		const observer = new QueryObserver(mounted.queryClient, {
			queryKey,
			queryFn,
		});
		cleanups.push(observer.subscribe(() => {}));

		mounted.windowTarget.dispatchEvent(new Event("focus"));
		mounted.documentTarget.dispatchEvent(new Event("visibilitychange"));
		mounted.windowTarget.dispatchEvent(new Event("online"));

		await expectRemoteWidgets(mounted);
		expect(queryFn).not.toHaveBeenCalled();
		expect(mounted.queryClient.getQueryData(queryKey)).toBe("cached board");
	});

	test("removes the resume listeners when the provider unmounts", async () => {
		vi.useFakeTimers();
		const mounted = mountedWidgetRefresh();
		mounted.windowTarget.dispatchEvent(new Event("focus"));
		mounted.unsubscribe();

		mounted.windowTarget.dispatchEvent(new Event("focus"));
		mounted.documentTarget.dispatchEvent(new Event("visibilitychange"));
		mounted.windowTarget.dispatchEvent(new Event("online"));
		await vi.advanceTimersByTimeAsync(100);

		for (const { queryKey, queryFn } of mounted.queries) {
			expect(queryFn).not.toHaveBeenCalled();
			expect(mounted.queryClient.getQueryData(queryKey)).toBe(
				"local widget cache",
			);
		}
	});

	test("reconnect restarts a refresh that stalled before the connection recovered", async () => {
		const mounted = mountedWidgetRefresh();
		for (const { queryFn } of mounted.queries) {
			queryFn.mockImplementationOnce(() => new Promise(() => {}));
		}
		mounted.windowTarget.dispatchEvent(new Event("focus"));
		await vi.waitFor(() => {
			for (const { queryFn } of mounted.queries) {
				expect(queryFn).toHaveBeenCalledTimes(1);
			}
		});

		mounted.windowTarget.dispatchEvent(new Event("online"));

		await vi.waitFor(() => {
			for (const { queryKey, queryFn } of mounted.queries) {
				expect(queryFn).toHaveBeenCalledTimes(2);
				expect(mounted.queryClient.getQueryData(queryKey)).toBe(
					"remote widgets",
				);
			}
		});
	});
});
