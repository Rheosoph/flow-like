/**
 * Tests for the pure date/geometry helpers shared by the Calendar and Gantt
 * A2UI components.
 */
import { describe, expect, test } from "bun:test";
import {
	densityPreset,
	eventDurationMinutes,
	eventEnd,
	ganttRange,
	genId,
	getMonthWeeks,
	getWeekDays,
	layoutOverlappingEvents,
	normalizeCalendarEvents,
	normalizeGanttTasks,
	parseTimeToMinutes,
	reorderById,
	taskBarDays,
	toDate,
	toDateInput,
	toDateTimeLocalInput,
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
	test("parses date-only strings as local midnight (no UTC shift)", () => {
		const d = toDate("2026-01-06");
		expect(d.getFullYear()).toBe(2026);
		expect(d.getMonth()).toBe(0);
		expect(d.getDate()).toBe(6);
		expect(d.getHours()).toBe(0);
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
	test("keeps object metadata and drops non-object metadata", () => {
		const [withMeta] = normalizeCalendarEvents([
			{
				id: "a",
				title: "A",
				start: "2026-01-06",
				metadata: { ticket: "FL-42" },
			},
		]);
		expect(withMeta.metadata).toEqual({ ticket: "FL-42" });
		const [scalarMeta] = normalizeGanttTasks([
			{ id: "t", name: "T", start: "2026-01-01", metadata: "raw-string" },
		]);
		expect(scalarMeta.metadata).toBeUndefined();
	});
	test("carries the optional link field through", () => {
		const [ev] = normalizeCalendarEvents([
			{ id: "a", title: "A", start: "2026-01-06", link: "/orders/42" },
		]);
		expect(ev.link).toBe("/orders/42");
		const [task] = normalizeGanttTasks([
			{
				id: "t",
				name: "T",
				start: "2026-01-01",
				link: "https://example.com",
			},
		]);
		expect(task.link).toBe("https://example.com");
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

	test("tolerates reversed task start/end without producing start > end", () => {
		const reversed = normalizeGanttTasks([
			{ id: "r", name: "R", start: "2026-01-20", end: "2026-01-10" },
		]);
		const range = ganttRange(reversed);
		expect(range.start.getTime()).toBeLessThanOrEqual(range.end.getTime());
		expect(range.totalDays).toBeGreaterThan(0);
		// eachDayOfInterval would throw on an inverted range — assert it doesn't.
		expect(() => taskBarDays(reversed[0], range)).not.toThrow();
	});
});

describe("layoutOverlappingEvents", () => {
	const ev = (id: string, start: string, end: string) => ({
		id,
		title: id,
		start,
		end,
	});

	test("non-overlapping events each get a full-width column", () => {
		const layout = layoutOverlappingEvents([
			ev("a", "2026-01-06T09:00:00", "2026-01-06T10:00:00"),
			ev("b", "2026-01-06T11:00:00", "2026-01-06T12:00:00"),
		]);
		expect(layout.get("a")).toEqual({ column: 0, columns: 1 });
		expect(layout.get("b")).toEqual({ column: 0, columns: 1 });
	});

	test("two overlapping events split into two columns", () => {
		const layout = layoutOverlappingEvents([
			ev("a", "2026-01-06T09:00:00", "2026-01-06T11:00:00"),
			ev("b", "2026-01-06T10:00:00", "2026-01-06T12:00:00"),
		]);
		expect(layout.get("a")).toEqual({ column: 0, columns: 2 });
		expect(layout.get("b")).toEqual({ column: 1, columns: 2 });
	});

	test("column is reused once freed within a cluster", () => {
		const layout = layoutOverlappingEvents([
			ev("a", "2026-01-06T09:00:00", "2026-01-06T10:00:00"),
			ev("b", "2026-01-06T09:30:00", "2026-01-06T12:00:00"),
			ev("c", "2026-01-06T10:30:00", "2026-01-06T11:30:00"),
		]);
		expect(layout.get("a")?.column).toBe(0);
		expect(layout.get("b")?.column).toBe(1);
		expect(layout.get("c")?.column).toBe(0); // a's column freed at 10:00
		expect(layout.get("c")?.columns).toBe(2);
	});

	test("clusters are independent", () => {
		const layout = layoutOverlappingEvents([
			ev("a", "2026-01-06T08:00:00", "2026-01-06T09:00:00"),
			ev("b", "2026-01-06T08:30:00", "2026-01-06T09:30:00"),
			ev("solo", "2026-01-06T14:00:00", "2026-01-06T15:00:00"),
		]);
		expect(layout.get("a")?.columns).toBe(2);
		expect(layout.get("solo")).toEqual({ column: 0, columns: 1 });
	});
});

describe("reorderById", () => {
	const list = [{ id: "a" }, { id: "b" }, { id: "c" }, { id: "d" }];
	test("moves an item forward and backward", () => {
		expect(reorderById(list, "a", "c").map((t) => t.id)).toEqual([
			"b",
			"c",
			"a",
			"d",
		]);
		expect(reorderById(list, "d", "b").map((t) => t.id)).toEqual([
			"a",
			"d",
			"b",
			"c",
		]);
	});
	test("returns the same list for unknown or identical ids", () => {
		expect(reorderById(list, "x", "b")).toBe(list);
		expect(reorderById(list, "b", "b")).toBe(list);
	});
});

describe("densityPreset", () => {
	test("maps known values and falls back to default", () => {
		expect(densityPreset("compact").rowHeight).toBeLessThan(
			densityPreset("default").rowHeight,
		);
		expect(densityPreset("comfortable").hourHeight).toBeGreaterThan(
			densityPreset("default").hourHeight,
		);
		expect(densityPreset("bogus")).toEqual(densityPreset("default"));
		expect(densityPreset(undefined)).toEqual(densityPreset("default"));
	});
});

describe("input formatting", () => {
	test("toDateTimeLocalInput / toDateInput format local time", () => {
		const d = new Date(2026, 0, 6, 9, 5);
		expect(toDateTimeLocalInput(d)).toBe("2026-01-06T09:05");
		expect(toDateInput(d)).toBe("2026-01-06");
	});
});

describe("genId", () => {
	test("is prefixed and unique", () => {
		const a = genId("task");
		const b = genId("task");
		expect(a.startsWith("task-")).toBe(true);
		expect(a).not.toBe(b);
	});
});
