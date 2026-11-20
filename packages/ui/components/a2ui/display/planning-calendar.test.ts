import { describe, expect, test } from "bun:test";
import { planningCalendarRange } from "./planning-calendar";

const focus = new Date(2026, 10, 20, 15, 30);

function days(range: { from: Date; to: Date }) {
	return [range.from, range.to].map((date) => date.toDateString());
}

describe("the days a view shows", () => {
	test("a month covers the whole weeks around it", () => {
		expect(days(planningCalendarRange("month", focus, 1))).toEqual([
			new Date(2026, 9, 26).toDateString(),
			new Date(2026, 11, 7).toDateString(),
		]);
		expect(days(planningCalendarRange("month", focus, 0))).toEqual([
			new Date(2026, 10, 1).toDateString(),
			new Date(2026, 11, 6).toDateString(),
		]);
	});

	test("a week, a day and the 14-day agenda", () => {
		expect(days(planningCalendarRange("week", focus, 1))).toEqual([
			new Date(2026, 10, 16).toDateString(),
			new Date(2026, 10, 23).toDateString(),
		]);
		expect(days(planningCalendarRange("day", focus, 1))).toEqual([
			new Date(2026, 10, 20).toDateString(),
			new Date(2026, 10, 21).toDateString(),
		]);
		expect(days(planningCalendarRange("agenda", focus, 1))).toEqual([
			new Date(2026, 10, 20).toDateString(),
			new Date(2026, 11, 4).toDateString(),
		]);
	});

	test("starts at local midnight", () => {
		const { from } = planningCalendarRange("day", focus, 1);
		expect([from.getHours(), from.getMinutes()]).toEqual([0, 0]);
	});
});
