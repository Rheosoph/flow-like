import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IUsageState } from "../../state/backend-state/usage-state";
import {
	homeExecutionActivityQuery,
	homeRecentExecutionsQuery,
	refreshHomeUsageQuery,
} from "./home-usage-query";

const scope = ["https://hub.example", "profile-a", "user-a"];
const activity = { total: 1 };
let client: QueryClient;
let unsubscribe: (() => void)[];

beforeEach(() => {
	vi.useFakeTimers();
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	client.mount();
	unsubscribe = [];
});

afterEach(() => {
	for (const stop of unsubscribe) stop();
	client.unmount();
	client.clear();
	vi.useRealTimers();
});

describe("home usage request sharing", () => {
	it("shares activity across widgets and polls once per five-minute interval", async () => {
		const getExecutionActivity = vi.fn(async () => activity);
		const usage = { getExecutionActivity } as unknown as IUsageState;
		const first = new QueryObserver(
			client,
			homeExecutionActivityQuery(usage, scope, 7),
		);
		const second = new QueryObserver(
			client,
			homeExecutionActivityQuery(usage, scope, 7),
		);
		unsubscribe.push(
			first.subscribe(() => {}),
			second.subscribe(() => {}),
		);
		await vi.advanceTimersByTimeAsync(0);
		expect(getExecutionActivity).toHaveBeenCalledTimes(1);
		expect(getExecutionActivity).toHaveBeenLastCalledWith(7, undefined, {
			fresh: true,
		});
		await vi.advanceTimersByTimeAsync(60_000);
		expect(getExecutionActivity).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(240_000);
		expect(getExecutionActivity).toHaveBeenCalledTimes(2);
		expect(getExecutionActivity).toHaveBeenLastCalledWith(7, undefined, {
			fresh: false,
		});
		expect(first.getCurrentResult().data).toEqual(activity);
		expect(second.getCurrentResult().data).toEqual(activity);
		first.setOptions(homeExecutionActivityQuery(usage, scope, 7, "", true));
		second.setOptions(homeExecutionActivityQuery(usage, scope, 7, "", true));
		await vi.advanceTimersByTimeAsync(300_000);
		expect(getExecutionActivity).toHaveBeenCalledTimes(2);
	});

	it("reuses fresh data on remount and keeps users, apps and periods separate", async () => {
		const getExecutionActivity = vi.fn(async () => activity);
		const usage = { getExecutionActivity } as unknown as IUsageState;
		await client.fetchQuery(homeExecutionActivityQuery(usage, scope, 7));
		await vi.advanceTimersByTimeAsync(60_000);
		await client.fetchQuery(homeExecutionActivityQuery(usage, scope, 7));
		expect(getExecutionActivity).toHaveBeenCalledTimes(1);
		await client.fetchQuery(homeExecutionActivityQuery(usage, scope, 30));
		await client.fetchQuery(
			homeExecutionActivityQuery(usage, scope, 7, "app-a"),
		);
		await client.fetchQuery(
			homeExecutionActivityQuery(usage, [...scope.slice(0, 2), "user-b"], 7),
		);
		expect(getExecutionActivity).toHaveBeenCalledTimes(4);
	});

	it("explicit refresh bypasses fresh local and server snapshots immediately", async () => {
		let total = 1;
		const getExecutionActivity = vi.fn(async () => ({ total: total++ }));
		const usage = { getExecutionActivity } as unknown as IUsageState;
		const options = homeExecutionActivityQuery(usage, scope, 7);
		const observer = new QueryObserver(client, options);
		unsubscribe.push(observer.subscribe(() => {}));
		await vi.advanceTimersByTimeAsync(0);
		await vi.advanceTimersByTimeAsync(10_000);
		const refreshed = await refreshHomeUsageQuery(
			client,
			options.queryKey,
			() => observer.refetch(),
		);
		expect(refreshed.data?.total).toBe(2);
		expect(getExecutionActivity).toHaveBeenCalledTimes(2);
		expect(getExecutionActivity).toHaveBeenLastCalledWith(7, undefined, {
			fresh: true,
		});
		await vi.advanceTimersByTimeAsync(300_000);
		expect(getExecutionActivity).toHaveBeenCalledTimes(3);
		expect(getExecutionActivity).toHaveBeenLastCalledWith(7, undefined, {
			fresh: false,
		});
	});

	it("a page reload asks the server for current data even within five minutes", async () => {
		const getExecutionActivity = vi.fn(async () => activity);
		const usage = { getExecutionActivity } as unknown as IUsageState;
		await client.fetchQuery(homeExecutionActivityQuery(usage, scope, 7));
		const reloaded = new QueryClient();
		try {
			await reloaded.fetchQuery(homeExecutionActivityQuery(usage, scope, 7));
			expect(getExecutionActivity.mock.calls).toEqual([
				[7, undefined, { fresh: true }],
				[7, undefined, { fresh: true }],
			]);
		} finally {
			reloaded.clear();
		}
	});

	it("recent widgets fetch only their bounded records and retain limit isolation", async () => {
		const getRecentExecutions = vi.fn(async () => ({
			items: [{ id: "run-a" }],
		}));
		const getExecutionHistory = vi.fn();
		const usage = {
			getRecentExecutions,
			getExecutionHistory,
		} as unknown as IUsageState;
		const first = await client.fetchQuery(
			homeRecentExecutionsQuery(usage, scope, "app-a", 8),
		);
		await client.fetchQuery(
			homeRecentExecutionsQuery(usage, scope, "app-a", 8),
		);
		await client.fetchQuery(
			homeRecentExecutionsQuery(usage, scope, "app-a", 20),
		);
		expect(first.items).toEqual([{ id: "run-a" }]);
		expect(getRecentExecutions.mock.calls).toEqual([
			[8, "app-a", { fresh: true }],
			[20, "app-a", { fresh: true }],
		]);
		expect(getExecutionHistory).not.toHaveBeenCalled();
	});
});
