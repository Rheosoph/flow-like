import { expect, test } from "bun:test";
import {
	type ScheduleDetail,
	deviceSchedule,
	knownZone,
	nextRuns,
	onceAhead,
} from "./schedule";

/* The shared expression list of run-more-design §7.1 row 1; the agent's test carries the same literals. */

const NOW = Date.UTC(2026, 9, 2, 12, 0, 0);
const iso = (ms: number) => new Date(ms).toISOString();
const read = (config: Record<string, unknown>) => deviceSchedule(config, NOW);
const scheduleOf = (config: Record<string, unknown>) => {
	const result = read(config);
	if (!result.ok) throw new Error(`expected a schedule, got ${result.problem}`);
	if (!result.schedule) throw new Error("expected a repeating schedule");
	return result.schedule;
};
const problemOf = (config: Record<string, unknown>) => {
	const result = read(config);
	return result.ok ? null : result.problem;
};

test("weekdays at nine run Monday to Friday at 09:00 local, with or without a seconds field", () => {
	for (const expression of ["0 0 9 * * 1-5", "0 9 * * 1-5"]) {
		const schedule = scheduleOf({ expression, timezone: "Europe/Berlin" });
		expect(schedule).toEqual({
			expression,
			timezone: "Europe/Berlin",
			zoneSet: true,
		});
		// 2026-10-02 is a Friday, 14:00 in Berlin: the next runs are Monday to Friday of the following week.
		expect(nextRuns(schedule, NOW, 6).map(iso)).toEqual([
			"2026-10-05T07:00:00.000Z",
			"2026-10-06T07:00:00.000Z",
			"2026-10-07T07:00:00.000Z",
			"2026-10-08T07:00:00.000Z",
			"2026-10-09T07:00:00.000Z",
			"2026-10-12T07:00:00.000Z",
		]);
	}
});

test("weekday 0 and weekday 7 are both Sunday", () => {
	for (const expression of ["0 0 18 * * 0", "0 0 18 * * 7"])
		expect(nextRuns(scheduleOf({ expression }), NOW, 2).map(iso)).toEqual([
			"2026-10-04T18:00:00.000Z",
			"2026-10-11T18:00:00.000Z",
		]);
});

test("a day of the month and a weekday together match either", () => {
	expect(
		nextRuns(scheduleOf({ expression: "0 0 9 1 * 1" }), NOW, 6).map(iso),
	).toEqual([
		"2026-10-05T09:00:00.000Z",
		"2026-10-12T09:00:00.000Z",
		"2026-10-19T09:00:00.000Z",
		"2026-10-26T09:00:00.000Z",
		"2026-11-01T09:00:00.000Z",
		"2026-11-02T09:00:00.000Z",
	]);
});

test("the expression and zone are read from every key the sink layer accepts", () => {
	for (const key of [
		"expression",
		"cron_expression",
		"cronExpression",
		"cron",
		"schedule",
	])
		expect(scheduleOf({ [key]: "0 0 2 * * *" }).expression).toBe("0 0 2 * * *");
	for (const key of ["timezone", "tz", "cron_timezone", "cronTimezone"])
		expect(
			scheduleOf({ expression: "0 0 2 * * *", [key]: "Europe/Berlin" }),
		).toMatchObject({ timezone: "Europe/Berlin", zoneSet: true });
	expect(
		scheduleOf({ expression: "0 0 2 * * *", cron: "0 0 3 * * *" }).expression,
	).toBe("0 0 2 * * *");
});

test("without a saved zone a device runs the schedule in UTC", () => {
	const schedule = scheduleOf({ expression: "0 0 2 * * *" });
	expect(schedule).toEqual({
		expression: "0 0 2 * * *",
		timezone: "UTC",
		zoneSet: false,
	});
	expect(nextRuns(schedule, NOW, 1).map(iso)).toEqual([
		"2026-10-03T02:00:00.000Z",
	]);
	expect(
		nextRuns(
			scheduleOf({ expression: "0 0 2 * * *", timezone: "Europe/Berlin" }),
			NOW,
			1,
		).map(iso),
	).toEqual(["2026-10-03T00:00:00.000Z"]);
});

test("month and weekday names are read in any case and upper-cased", () => {
	expect(scheduleOf({ expression: " 0 0 9 * jan,Feb  mon-FRI " })).toEqual({
		expression: "0 0 9 * JAN,FEB MON-FRI",
		timezone: "UTC",
		zoneSet: false,
	});
});

test("what a device does not read is invalid, with what was unexpected", () => {
	const invalid: [Record<string, unknown>, ScheduleDetail][] = [
		[
			{ expression: "0 0 9 * * 1-5", timezone: "Mars/Olympus" },
			{ code: "zone", zone: "Mars/Olympus" },
		],
		[
			{ expression: "0 0 9 * * 1-5", timezone: "+02:00" },
			{ code: "zone", zone: "+02:00" },
		],
		[{ expression: "0 9 * *" }, { code: "fields", count: 4 }],
		[{ expression: "0 0 9 * * 1-5 2027" }, { code: "fields", count: 7 }],
		[{ expression: "0 0 9 ? * 1-5" }, { code: "syntax", field: "?" }],
		[{ expression: "0 0 9 L * *" }, { code: "syntax", field: "L" }],
		[{ expression: "0 0 9 15W * *" }, { code: "syntax", field: "15W" }],
		[{ expression: "0 0 9 * * 1#2" }, { code: "syntax", field: "1#2" }],
		[{ expression: "@daily" }, { code: "fields", count: 1 }],
		[{ expression: "0 0 nine * * *" }, { code: "syntax", field: "nine" }],
		[{ expression: "0 0 9 * MON *" }, { code: "syntax", field: "MON" }],
		[{ expression: "0 0 9 * * JAN" }, { code: "syntax", field: "JAN" }],
		[{ expression: "0 0 0 30 2 *" }, { code: "parser" }],
		[{ expression: "0 0 25 * * *" }, { code: "parser" }],
		[{ expression: "60 0 9 * * *" }, { code: "parser" }],
		[{ expression: `0 0 9 ${"1,".repeat(64)}1 * *` }, { code: "length" }],
	];
	for (const [config, detail] of invalid)
		expect(read(config)).toEqual({
			ok: false,
			problem: "schedule_invalid",
			detail,
		});
});

test("a seconds field that is not one number runs more often than once a minute", () => {
	for (const expression of [
		"*/30 * * * * *",
		"* * * * * *",
		"0,30 * * * * *",
		"0/1 * * * * *",
		"0-5 0 9 * * *",
	])
		expect(read({ expression })).toEqual({
			ok: false,
			problem: "schedule_too_often",
		});
	expect(problemOf({ expression: "30 * * * * *" })).toBeNull();
	expect(problemOf({ expression: "* * * * *" })).toBeNull();
});

test("one-time schedules, both forms at once and no schedule each have their own reason", () => {
	const once = { date: "2026-12-24", time: "18:00" };
	expect(read({ scheduled_for: once })).toEqual({
		ok: true,
		once: {
			date: "2026-12-24",
			time: "18:00",
			at: Date.UTC(2026, 11, 24, 18) / 1000,
			timezone: "UTC",
			zoneSet: false,
		},
	});
	expect(problemOf({ scheduledFor: once, expression: null })).toBeNull();
	expect(read({ scheduled_for: once, expression: "0 0 9 * * *" })).toEqual({
		ok: false,
		problem: "schedule_invalid",
		detail: { code: "both" },
	});
	for (const config of [
		{},
		{ expression: "" },
		{ expression: "   " },
		{ expression: null },
		{ timezone: "Europe/Berlin" },
		{ scheduled_for: { date: "2026-12-24" } },
	])
		expect(problemOf(config)).toBe("schedule_missing");
	expect(deviceSchedule(null, NOW)).toEqual({
		ok: false,
		problem: "schedule_missing",
	});
	expect(deviceSchedule(undefined, NOW)).toEqual({
		ok: false,
		problem: "schedule_missing",
	});
});

test("sink_execution, last_fired and payload do not change the answer", () => {
	expect(
		scheduleOf({
			expression: "0 0 2 * * *",
			timezone: "Europe/Berlin",
			sink_execution: "LOCAL",
			last_fired: "2026-10-01T00:00:00Z",
			payload: { report: "nightly" },
		}),
	).toEqual({
		expression: "0 0 2 * * *",
		timezone: "Europe/Berlin",
		zoneSet: true,
	});
});

test("next runs follow the zone across a daylight-saving change and end quietly", () => {
	const nightly = { expression: "0 30 2 * * *", timezone: "Europe/Berlin" };
	// Clocks go back on 2026-10-25: 02:30 local occurs twice and runs once.
	expect(nextRuns(nightly, Date.UTC(2026, 9, 24, 12), 3).map(iso)).toEqual([
		"2026-10-25T00:30:00.000Z",
		"2026-10-26T01:30:00.000Z",
		"2026-10-27T01:30:00.000Z",
	]);
	expect(nextRuns(nightly, NOW, 0)).toEqual([]);
	expect(
		nextRuns({ expression: "not a schedule", timezone: "UTC" }, NOW, 3),
	).toEqual([]);
	expect(
		nextRuns({ expression: "0 0 2 * * *", timezone: "Mars/Olympus" }, NOW, 3),
	).toEqual([]);
});

/* The one-time table of run-more-2-design §1.3; the agent's test carries the same literals. */

const ONCE_CASES: [string, string, string | null, string | ScheduleDetail][] = [
	["2026-08-15", "09:30", "Europe/Berlin", "2026-08-15T07:30:00.000Z"],
	["2026-10-25", "02:30", "Europe/Berlin", "2026-10-25T00:30:00.000Z"],
	[
		"2027-03-28",
		"02:30",
		"Europe/Berlin",
		{
			code: "gap",
			date: "2027-03-28",
			time: "02:30",
			zone: "Europe/Berlin",
		},
	],
	["2027-03-28", "03:00", "Europe/Berlin", "2027-03-28T01:00:00.000Z"],
	[
		"2026-10-04",
		"02:15",
		"Australia/Lord_Howe",
		{
			code: "gap",
			date: "2026-10-04",
			time: "02:15",
			zone: "Australia/Lord_Howe",
		},
	],
	["2026-08-15", "09:30", null, "2026-08-15T09:30:00.000Z"],
	["2026-02-30", "09:30", null, { code: "date", date: "2026-02-30" }],
	["2026-08-15", "9:30", null, { code: "time", time: "9:30" }],
	["2026-08-15", "09:30:00", null, { code: "time", time: "09:30:00" }],
	["2026-08-15", "24:00", null, { code: "time", time: "24:00" }],
	["1999-12-31", "23:59", null, { code: "range", date: "1999-12-31" }],
];

test("a one-time schedule is one instant in its zone: the earlier of two, none in a gap", () => {
	for (const [date, time, zone, expected] of ONCE_CASES) {
		const result = read({
			scheduled_for: { date, time },
			...(zone ? { timezone: zone } : {}),
		});
		if (typeof expected === "string") {
			if (!result.ok || !result.once)
				throw new Error(`${date} ${time} ${zone}: ${JSON.stringify(result)}`);
			expect(iso(result.once.at * 1000)).toBe(expected);
			expect(result.once).toMatchObject({
				date,
				time,
				timezone: zone ?? "UTC",
				zoneSet: zone !== null,
			});
		} else
			expect(result).toEqual({
				ok: false,
				problem: "schedule_invalid",
				detail: expected,
			});
	}
});

test("a one-time schedule is valid whatever the clock says; whether its time is ahead is asked apart", () => {
	const passed = read({ scheduled_for: { date: "2026-01-01", time: "00:00" } });
	expect(passed.ok).toBe(true);
	if (!passed.ok || !passed.once)
		throw new Error("expected a one-time schedule");
	expect(onceAhead(passed.once, NOW)).toBe(false);
	expect(onceAhead(passed.once, Date.UTC(2025, 11, 31, 23, 59))).toBe(true);
	expect(onceAhead({ at: NOW / 1000 }, NOW)).toBe(false);
	expect(
		read({
			scheduled_for: { date: "2026-08-15", time: "09:30" },
			timezone: "Mars/Olympus",
		}),
	).toEqual({
		ok: false,
		problem: "schedule_invalid",
		detail: { code: "zone", zone: "Mars/Olympus" },
	});
});

test("a zone a device refuses is refused here though Intl takes it", () => {
	for (const zone of ["+02:00", "-05", "utc", "europe/berlin", "EUROPE/BERLIN"])
		expect(knownZone(zone)).toBe(false);
	for (const zone of ["UTC", "Europe/Berlin", "Etc/UTC", "America/New_York"])
		expect(knownZone(zone)).toBe(true);
	expect(read({ expression: "0 0 9 * * *", timezone: "utc" })).toEqual({
		ok: false,
		problem: "schedule_invalid",
		detail: { code: "zone", zone: "utc" },
	});
});
