import { describe, expect, it } from "vitest";
import type {
	IExecutionActivity,
	IExecutionUsageRecord,
} from "../../lib/schema/usage/tracking";
import {
	hasAttentionSeverity,
	homeActivityCoverage,
	homeActivityDays,
	homeActivityPeriod,
	homeActivitySourceLabel,
	homeDurationMs,
	homeUsageDollars,
	normalizeHomeActivity,
} from "./home-activity-statistics";

const flagged = (id: string): IExecutionUsageRecord => ({
	id,
	created_at: "2026-09-11T10:00:00Z",
	status: "Error",
	app_id: "app-a",
	instance: null,
	board_id: "board",
	node_id: "node",
	version: "1",
	microseconds: 1000,
	technical_user_id: null,
});

const response = (
	overrides: Partial<IExecutionActivity> = {},
): IExecutionActivity => ({
	days: 7,
	from: "2026-09-05T00:00:00Z",
	to: "2026-09-11T14:00:00Z",
	buckets: [
		{ day: "2026-09-10", count: 4_000, attention_count: 12 },
		{ day: "2026-09-11", count: 861, attention_count: 3 },
	],
	apps: [
		{ app_id: "app-a", count: 4_800, attention_count: 15 },
		{ app_id: null, count: 61, attention_count: 0 },
	],
	total: 4_861,
	attention_total: 15,
	average_microseconds: 2_500,
	attention: [flagged("a"), flagged("b")],
	...overrides,
});

describe("home execution activity", () => {
	it("renames the wire shape without moving a number", () => {
		const activity = normalizeHomeActivity(response());
		expect(activity.total).toBe(4_861);
		expect(activity.attentionTotal).toBe(15);
		expect(activity.averageMicroseconds).toBe(2_500);
		expect(activity.buckets[0]).toEqual({
			day: "2026-09-10",
			count: 4_000,
			attentionCount: 12,
		});
		expect(activity.apps[1]).toEqual({
			appId: null,
			count: 61,
			attentionCount: 0,
		});
	});

	it("reports a flagged list shorter than its own total as capped", () => {
		expect(normalizeHomeActivity(response()).attentionCapped).toBe(true);
		expect(
			normalizeHomeActivity(
				response({
					attention_total: 2,
					attention: [flagged("a"), flagged("b")],
				}),
			).attentionCapped,
		).toBe(false);
		expect(
			normalizeHomeActivity(response({ attention_total: 0, attention: [] }))
				.attentionCapped,
		).toBe(false);
	});

	it("survives a response missing its optional collections", () => {
		const activity = normalizeHomeActivity({
			days: 1,
			from: "2026-09-11T00:00:00Z",
			to: "2026-09-11T14:00:00Z",
			total: 0,
			attention_total: 0,
			average_microseconds: null,
		} as unknown as IExecutionActivity);
		expect(activity.buckets).toEqual([]);
		expect(activity.apps).toEqual([]);
		expect(activity.attention).toEqual([]);
		expect(activity.attentionCapped).toBe(false);
	});

	it("reads a garbled response as an empty period instead of throwing", () => {
		const activity = normalizeHomeActivity({
			days: 7,
			errorType: "Runtime.ExitError",
			buckets: { day: "2026-09-11" },
			apps: "unavailable",
			attention: [null],
		} as unknown as IExecutionActivity);
		expect(activity.buckets).toEqual([]);
		expect(activity.apps).toEqual([]);
		expect(activity.attention).toEqual([]);
		expect(activity.total).toBe(0);
		expect(activity.attentionTotal).toBe(0);
		expect(activity.averageMicroseconds).toBeNull();
		expect(homeActivitySourceLabel(activity)).toContain("0 records");
	});

	it("describes a counted period rather than a sampled one", () => {
		const coverage = homeActivityCoverage(normalizeHomeActivity(response()));
		expect(coverage).toContain("Last 7 days (UTC)");
		expect(coverage).toContain("4,861 execution records");
		expect(coverage).toContain("counted in full rather than sampled");
		expect(coverage).toContain("15");
		expect(coverage).not.toContain("sample counts");
	});

	it("keeps singular and plural record labels honest", () => {
		const one = normalizeHomeActivity(response({ total: 1, days: 1 }));
		expect(homeActivityCoverage(one)).toContain("1 execution record,");
		expect(homeActivityCoverage(one)).toContain("Today (UTC)");
		expect(homeActivitySourceLabel(one)).toBe(
			"Your account · 1 record · today (UTC)",
		);
		expect(homeActivitySourceLabel(normalizeHomeActivity(response()))).toBe(
			"Your account · 4,861 records · last 7 days (UTC)",
		);
	});

	it("uses bounded timeframes and converts the documented units", () => {
		expect(homeActivityDays(1)).toBe(1);
		expect(homeActivityDays(30)).toBe(30);
		expect(homeActivityDays(365)).toBe(7);
		expect(homeActivityPeriod(1)).toBe("Today");
		expect(homeActivityPeriod(30)).toBe("Last 30 days");
		expect(homeUsageDollars(1_500_000)).toContain("1.50");
		expect(homeDurationMs(2_500)).toBe("3 ms");
		expect(homeDurationMs(null)).toBe("No records");
	});

	it("treats only Error and Fatal as attention severity", () => {
		expect(hasAttentionSeverity("ERROR")).toBe(true);
		expect(hasAttentionSeverity("Fatal")).toBe(true);
		expect(hasAttentionSeverity("Warn")).toBe(false);
		expect(hasAttentionSeverity("success")).toBe(false);
		expect(hasAttentionSeverity(undefined as unknown as string)).toBe(false);
	});
});
