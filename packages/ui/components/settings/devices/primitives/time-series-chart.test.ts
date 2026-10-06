import { describe, expect, test } from "bun:test";
import {
	type ChartSeries,
	chartHasData,
	chartMax,
	timeTicks,
	valueTicks,
} from "./time-series-chart";

const HOUR = 3_600;
const DAY = 86_400;
/** Midnight UTC, 2026-10-01. */
const MIDNIGHT = 1_790_812_800;

const series = (
	id: string,
	values: (number | null)[],
	tone: ChartSeries["tone"] = "accent",
): ChartSeries => ({ id, label: id, tone, values });

describe("value ticks", () => {
	test("run from zero to a 1-2-5 step at or above the highest value", () => {
		expect(valueTicks(100)).toEqual([0, 50, 100]);
		expect(valueTicks(101)).toEqual([0, 50, 100, 150]);
		expect(valueTicks(7)).toEqual([0, 5, 10]);
		expect(valueTicks(3)).toEqual([0, 1, 2, 3]);
		expect(valueTicks(1_900)).toEqual([0, 1_000, 2_000]);
	});

	test("small fractions stay clean", () => {
		expect(valueTicks(0.3)).toEqual([0, 0.1, 0.2, 0.3]);
		expect(valueTicks(0.25)).toEqual([0, 0.1, 0.2, 0.3]);
	});

	test("nothing to show still gets an axis", () => {
		expect(valueTicks(0)).toEqual([0, 1]);
		expect(valueTicks(Number.NaN)).toEqual([0, 1]);
	});
});

describe("time ticks", () => {
	const utc = () => 0;

	test("a day gets clean six-hour marks", () => {
		const { every, ticks } = timeTicks(MIDNIGHT, MIDNIGHT + DAY, 6, utc);
		expect(every).toBe(6 * HOUR);
		expect(ticks).toEqual([0, 6, 12, 18, 24].map((h) => MIDNIGHT + h * HOUR));
	});

	test("marks fall on local clock times, not on UTC ones", () => {
		const berlin = () => 2 * HOUR;
		const { ticks } = timeTicks(MIDNIGHT, MIDNIGHT + DAY, 6, berlin);
		expect(ticks[0]).toBe(MIDNIGHT + 4 * HOUR);
		for (const at of ticks) expect((at + 2 * HOUR) % (6 * HOUR)).toBe(0);
	});

	test("a window that starts between marks begins at the next one", () => {
		const { ticks } = timeTicks(
			MIDNIGHT + 5 * HOUR,
			MIDNIGHT + 29 * HOUR,
			4,
			utc,
		);
		expect(ticks).toEqual([6, 12, 18, 24].map((h) => MIDNIGHT + h * HOUR));
	});

	test("longer windows step by days, weeks and months as width allows", () => {
		expect(timeTicks(MIDNIGHT, MIDNIGHT + 7 * DAY, 7, utc).every).toBe(DAY);
		expect(timeTicks(MIDNIGHT, MIDNIGHT + 7 * DAY, 3, utc).every).toBe(7 * DAY);
		expect(timeTicks(MIDNIGHT, MIDNIGHT + 90 * DAY, 7, utc).every).toBe(
			14 * DAY,
		);
		expect(timeTicks(MIDNIGHT, MIDNIGHT + 90 * DAY, 4, utc).every).toBe(
			30 * DAY,
		);
	});
});

describe("chart extent", () => {
	test("columns hold their stacked totals, lines their highest point", () => {
		const both = [series("a", [1, 4, null]), series("b", [2, 3, 9])];
		expect(chartMax("columns", both)).toBe(9);
		expect(chartMax("lines", both)).toBe(9);
		const stacked = [series("a", [5, 4]), series("b", [2, 3])];
		expect(chartMax("columns", stacked)).toBe(7);
		expect(chartMax("lines", stacked)).toBe(5);
	});

	test("columns need a value above zero, lines any reported value", () => {
		expect(chartHasData("columns", [series("a", [0, 0, null])])).toBe(false);
		expect(chartHasData("columns", [series("a", [0, 2])])).toBe(true);
		expect(chartHasData("lines", [series("a", [null, null])])).toBe(false);
		expect(chartHasData("lines", [series("a", [null, 0])])).toBe(true);
		expect(chartHasData("lines", [])).toBe(false);
	});
});
