import { describe, expect, test } from "bun:test";
import type { IHistoryEntry } from "./chat-history-types";
import { groupHistoryByDate } from "./group-history";

// Fixed reference clock: 2026-08-12T15:00:00 local time.
const NOW = new Date(2026, 7, 12, 15, 0, 0).getTime();
const DAY = 24 * 60 * 60 * 1000;

function entry(
	id: string,
	updatedAt: number,
	pinnedAt?: number,
): IHistoryEntry {
	return { id, title: id, updatedAt, pinnedAt };
}

describe("groupHistoryByDate", () => {
	test("buckets by calendar day, not by elapsed hours", () => {
		const groups = groupHistoryByDate(
			[
				entry("this-morning", new Date(2026, 7, 12, 1, 0).getTime()),
				entry("late-yesterday", new Date(2026, 7, 11, 23, 30).getTime()),
			],
			NOW,
		);

		// 14h ago is still "Today" and 15.5h ago is already "Yesterday": a pure hours-since-now
		// split would put both in the same bucket.
		expect(groups.map((g) => g.label)).toEqual(["Today", "Yesterday"]);
		expect(groups[0].entries.map((e) => e.id)).toEqual(["this-morning"]);
		expect(groups[1].entries.map((e) => e.id)).toEqual(["late-yesterday"]);
	});

	test("orders sections newest-first and covers every window", () => {
		const groups = groupHistoryByDate(
			[
				entry("ancient", NOW - 400 * DAY),
				entry("month", NOW - 20 * DAY),
				entry("week", NOW - 3 * DAY),
				entry("yesterday", NOW - DAY),
				entry("today", NOW - 60_000),
			],
			NOW,
		);

		expect(groups.map((g) => g.label)).toEqual([
			"Today",
			"Yesterday",
			"Previous 7 days",
			"Previous 30 days",
			"Older",
		]);
	});

	test("pinned entries lead, leave their date bucket, and sort by pin time", () => {
		const groups = groupHistoryByDate(
			[
				entry("old-pin", NOW - 400 * DAY, NOW - 10 * DAY),
				entry("today", NOW - 60_000),
				entry("fresh-pin", NOW - 2 * DAY, NOW - 60_000),
			],
			NOW,
		);

		expect(groups[0].label).toBe("Pinned");
		expect(groups[0].pinned).toBe(true);
		expect(groups[0].entries.map((e) => e.id)).toEqual([
			"fresh-pin",
			"old-pin",
		]);
		// A pinned conversation must appear exactly once, not also under its date.
		expect(groups.slice(1).flatMap((g) => g.entries.map((e) => e.id))).toEqual([
			"today",
		]);
	});

	test("empty buckets are omitted entirely", () => {
		expect(groupHistoryByDate([], NOW)).toEqual([]);
		expect(
			groupHistoryByDate([entry("today", NOW - 1000)], NOW).map((g) => g.label),
		).toEqual(["Today"]);
	});
});
