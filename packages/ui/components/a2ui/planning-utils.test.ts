/**
 * Tests for the pure date/geometry helpers shared by the Calendar and Gantt
 * A2UI components.
 */
import { describe, expect, test } from "bun:test";
import {
	eventDurationMinutes,
	eventEnd,
	ganttRange,
	getMonthWeeks,
	getWeekDays,
	normalizeCalendarEvents,
	normalizeGanttTasks,
	parseTimeToMinutes,
	taskBarDays,
	toDate,
} from "./planning-utils";

describe("toDate", () => {
	test("parses ISO strings", () => {
		expect(toDate("2026-01-06T10:00:00Z").getUTCFullYear()).toBe(2026);
	});
	test("parses date-only strings", () => {
		expect(toDate("2026-01-06").getFullYear()).toBe(2026);
	});
	test("never returns NaN for bad input", () => {
		expect(Number.isNaN(toDate("not-a-date").getTime())).toBe(false);
		expect(Number.isNaN(toDate(undefined).getTime())).toBe(false);
	});
	test("passes Date through", () => {
		const d = new Date(2026, 0, 1);
		expect(toDate(d)).toBe(d);
	});
});

describe("normalizeCalendarEvents", () => {
	test("coerces loose objects and fills ids/titles", () => {
		const events = normalizeCalendarEvents([
			{ start: "2026-01-06", name: "From name field" },
			{ id: "x", title: "Titled", start: "2026-01-07", allDay: "true" },
		]);
		expect(events).toHaveLength(2);
		expect(events[0].id).toBe("event-0");
		expect(events[0].title).toBe("From name field");
		expect(events[1].allDay).toBe(true);
	});
	test("parses a JSON string payload", () => {
		const events = normalizeCalendarEvents(
			JSON.stringify([{ id: "a", title: "A", start: "2026-01-06" }]),
		);
		expect(events).toHaveLength(1);
	});
	test("drops entries without a start", () => {
		expect(normalizeCalendarEvents([{ title: "no start" }])).toHaveLength(0);
	});
	test("returns [] for non-array/garbage", () => {
		expect(normalizeCalendarEvents(null)).toEqual([]);
		expect(normalizeCalendarEvents(42)).toEqual([]);
	});
});

describe("normalizeGanttTasks", () => {
	test("requires a start; end defaults to start when omitted", () => {
		expect(normalizeGanttTasks([{ id: "t", name: "T" }])).toHaveLength(0);
		const [same] = normalizeGanttTasks([
			{ id: "t", name: "T", start: "2026-01-01" },
		]);
		expect(same.end).toBe("2026-01-01");
		expect(
			normalizeGanttTasks([
				{ id: "t", name: "T", start: "2026-01-01", end: "2026-01-03" },
			]),
		).toHaveLength(1);
	});
	test("normalizes dependencies to string[]", () => {
		const [task] = normalizeGanttTasks([
			{
				id: "t2",
				name: "T2",
				start: "2026-01-01",
				end: "2026-01-02",
				dependencies: ["t1"],
			},
		]);
		expect(task.dependencies).toEqual(["t1"]);
	});
});

describe("eventEnd / eventDurationMinutes", () => {
	test("defaults to 1h for timed events without end", () => {
		const ev = { id: "e", title: "E", start: "2026-01-06T09:00:00Z" };
		expect(eventDurationMinutes(ev)).toBe(60);
	});
	test("honours explicit end", () => {
		const ev = {
			id: "e",
			title: "E",
			start: "2026-01-06T09:00:00Z",
			end: "2026-01-06T10:30:00Z",
		};
		expect(eventDurationMinutes(ev)).toBe(90);
		expect(eventEnd(ev).getUTCHours()).toBe(10);
	});
});

describe("getMonthWeeks / getWeekDays", () => {
	test("month grid is whole weeks of 7 days", () => {
		const weeks = getMonthWeeks(new Date(2026, 0, 15), 1);
		expect(weeks.length).toBeGreaterThanOrEqual(4);
		for (const week of weeks) expect(week).toHaveLength(7);
	});
	test("week has 7 days starting on the configured day", () => {
		const days = getWeekDays(new Date(2026, 0, 7), 1);
		expect(days).toHaveLength(7);
		expect(days[0].getDay()).toBe(1); // Monday
	});
});

describe("parseTimeToMinutes", () => {
	test("parses HH:MM", () => {
		expect(parseTimeToMinutes("06:30", 0)).toBe(390);
	});
	test("falls back on bad input", () => {
		expect(parseTimeToMinutes(undefined, 480)).toBe(480);
		expect(parseTimeToMinutes("nonsense", 120)).toBe(120);
	});
});

describe("ganttRange / taskBarDays", () => {
	const tasks = normalizeGanttTasks([
		{ id: "a", name: "A", start: "2026-01-05", end: "2026-01-09" },
		{ id: "b", name: "B", start: "2026-01-12", end: "2026-01-16" },
	]);

	test("range spans all tasks with padding", () => {
		const range = ganttRange(tasks, 2);
		expect(range.start.getTime()).toBeLessThan(toDate("2026-01-05").getTime());
		expect(range.end.getTime()).toBeGreaterThan(toDate("2026-01-16").getTime());
		expect(range.totalDays).toBeGreaterThan(0);
	});

	test("bar geometry is relative to the range start", () => {
		const range = ganttRange(tasks, 2);
		const geom = taskBarDays(tasks[0], range);
		expect(geom.offsetDays).toBe(2); // padded by 2 days
		expect(geom.spanDays).toBe(5); // Jan 5..9 inclusive
	});

	test("empty task list yields a valid default range", () => {
		const range = ganttRange([]);
		expect(range.totalDays).toBeGreaterThan(0);
	});
});
