import { describe, expect, test } from "bun:test";
import { dateToIso, isoToDate, startOf, weekStartOf } from "./date-iso";

describe("rail dates and calendar days", () => {
	test("a real day round-trips through local calendar parts", () => {
		for (const iso of [
			"2026-09-18",
			"2026-02-28",
			"2024-02-29",
			"2026-12-31",
			"0999-01-01",
		]) {
			const date = isoToDate(iso);
			expect(date).toBeDefined();
			expect(dateToIso(date as Date)).toBe(iso);
		}
	});

	test("anything that is not a real day is undefined", () => {
		for (const text of [
			"",
			"2026-02-30",
			"2026-13-01",
			"18/9/2026",
			"2026-9-18",
			"soon",
		]) {
			expect(isoToDate(text)).toBeUndefined();
		}
	});

	test("the calendar starts on the chosen day, else the anchor, else today", () => {
		expect(dateToIso(startOf("2026-09-17", "2026-09-30", "2026-10-05"))).toBe(
			"2026-09-17",
		);
		expect(dateToIso(startOf("", "2026-09-30", "2026-10-05"))).toBe(
			"2026-09-30",
		);
		expect(dateToIso(startOf("", "", "2026-10-05"))).toBe("2026-10-05");
	});

	test("the first day of the week is a getDay number, Monday when Intl cannot say", () => {
		for (const locale of [
			"en-GB",
			"en-US",
			"de-DE",
			"ja-JP",
			"not a locale!",
		]) {
			const first = weekStartOf(locale);
			expect([0, 1, 2, 3, 4, 5, 6]).toContain(first);
		}
		expect(weekStartOf("not a locale!")).toBe(1);
	});
});
