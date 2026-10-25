import { beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({ fetcher: vi.fn() }));
vi.mock("../../../lib/api", () => ({ fetcher: mocks.fetcher }));

import { UsageState } from "../usage-state";

const profile = { hub: "hub.example" };
const auth = { isAuthenticated: true, user: { access_token: "token" } };
const backend = { profile, auth } as unknown as TauriBackend;

beforeEach(() => vi.resetAllMocks());

describe("desktop recent execution requests", () => {
	test("requests a bounded list with authenticated profile context", async () => {
		mocks.fetcher.mockResolvedValue({ items: [{ id: "run-a" }] });
		expect(
			await new UsageState(backend).getRecentExecutions(8, "app/a"),
		).toEqual({
			items: [{ id: "run-a" }],
		});
		expect(mocks.fetcher).toHaveBeenCalledWith(
			profile,
			"usage/executions/recent?limit=8&app_id=app%2Fa",
			{ method: "GET" },
			auth,
		);
	});

	test("normalizes an empty list without manufacturing a total", async () => {
		mocks.fetcher.mockResolvedValue({ items: null });
		expect(await new UsageState(backend).getRecentExecutions()).toEqual({
			items: [],
		});
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"usage/executions/recent?limit=50",
		);
	});

	test("explicit refresh forwards the cache bypass for every execution usage read", async () => {
		mocks.fetcher.mockResolvedValue({ items: [] });
		const usage = new UsageState(backend);
		await usage.getExecutionHistory(0, 50, undefined, { fresh: true });
		await usage.getRecentExecutions(8, undefined, { fresh: true });
		await usage.getExecutionActivity(7, undefined, { fresh: true });
		await usage.getUsageSummary({ fresh: true });
		for (const call of mocks.fetcher.mock.calls) {
			expect(call[2]).toEqual({
				method: "GET",
				headers: { "Cache-Control": "no-cache" },
			});
		}
		await usage.getExecutionActivity(7, undefined, { fresh: false });
		expect(mocks.fetcher.mock.lastCall?.[2]).toEqual({ method: "GET" });
	});

	test("propagates source errors so the widget can retry", async () => {
		const error = new Error("unavailable");
		mocks.fetcher.mockRejectedValue(error);
		await expect(new UsageState(backend).getRecentExecutions()).rejects.toBe(
			error,
		);
	});
});
