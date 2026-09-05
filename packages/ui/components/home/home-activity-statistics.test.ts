import { describe, expect, it } from "vitest";
import type { IExecutionUsageRecord } from "../../lib/schema/usage/tracking";
import {
	hasAttentionSeverity,
	homeActivityCoverage,
	homeActivityDays,
	homeUsageDollars,
	summarizeHomeExecutions,
} from "./home-activity-statistics";

const now = Date.parse("2026-09-05T14:00:00Z");
const run = (
	id: string,
	created_at: string,
	status = "Info",
	app_id: string | null = "app-a",
): IExecutionUsageRecord => ({
	id,
	created_at,
	status,
	app_id,
	instance: null,
	board_id: "board",
	node_id: "node",
	version: "1",
	microseconds: 1000,
	technical_user_id: null,
});

function summarize(
	items: IExecutionUsageRecord[],
	days: unknown = 7,
	total = items.length,
) {
	return summarizeHomeExecutions(
		{ items, total, page: 0, page_size: 100 },
		days,
		now,
	);
}

describe("home execution statistics", () => {
	it("groups UTC days, excludes older/future/invalid dates and deduplicates records", () => {
		const data = summarize([
			run("first", "2026-08-30T00:00:00Z", "Error"),
			run("last", "2026-09-05T13:00:00Z", "Warn"),
			run("last", "2026-09-05T13:00:00Z", "Warn"),
			run("old", "2026-08-29T23:59:59Z"),
			run("future", "2026-09-05T15:00:00Z"),
			run("invalid", "invalid"),
		]);
		expect(data.rows.map((item) => item.id)).toEqual(["last", "first"]);
		expect(data.buckets).toHaveLength(7);
		expect(data.buckets[0]).toEqual({
			day: "2026-08-30",
			count: 1,
			attentionCount: 1,
		});
		expect(data.buckets[6]).toEqual({
			day: "2026-09-05",
			count: 1,
			attentionCount: 0,
		});
		expect(data.invalidDates).toBe(1);
	});

	it("keeps missing app associations and treats only Error/Fatal as attention severity", () => {
		const data = summarize([
			run("a", "2026-09-05T10:00:00Z", "Debug"),
			run("b", "2026-09-05T11:00:00Z", "Fatal", null),
			run("c", "2026-09-05T12:00:00Z", "Warn"),
		]);
		expect(data.apps).toEqual([
			{ appId: "app-a", count: 2, attentionCount: 0 },
			{ appId: null, count: 1, attentionCount: 1 },
		]);
		expect(hasAttentionSeverity("ERROR")).toBe(true);
		expect(hasAttentionSeverity("Warn")).toBe(false);
		expect(hasAttentionSeverity("success")).toBe(false);
	});

	it("labels bounded samples even if their period contains no records", () => {
		const data = summarize([run("old", "2026-08-01T00:00:00Z")], 1, 500);
		expect(data.partial).toBe(true);
		expect(homeActivityCoverage(data)).toContain(
			"0 records in the latest 1 of 500",
		);
		expect(homeActivityCoverage(data)).toContain("may omit earlier activity");
		expect(homeActivityCoverage(summarize([], 1))).toContain("All available");
	});

	it("uses bounded timeframes and converts the documented microdollar unit", () => {
		expect(homeActivityDays(1)).toBe(1);
		expect(homeActivityDays(30)).toBe(30);
		expect(homeActivityDays(365)).toBe(7);
		expect(homeUsageDollars(1_500_000)).toContain("1.50");
	});
});
