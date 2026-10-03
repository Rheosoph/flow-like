import { expect, test } from "bun:test";
import {
	type InstantProblem,
	ONCE_MAX_AT,
	ONCE_MIN_AT,
	intlZone,
	resolveLocalInstant,
} from "./schedule-instant";

const at = (iso: string) => Date.parse(iso) / 1000;

/* The one-time table of run-more-2-design §1.3 ("no zone" is UTC); the agent's test carries the same literals. */
const CASES: [string, string, string, number | InstantProblem][] = [
	["2026-08-15", "09:30", "Europe/Berlin", at("2026-08-15T07:30:00Z")],
	["2026-10-25", "02:30", "Europe/Berlin", at("2026-10-25T00:30:00Z")],
	["2027-03-28", "02:30", "Europe/Berlin", "gap"],
	["2027-03-28", "03:00", "Europe/Berlin", at("2027-03-28T01:00:00Z")],
	["2026-10-04", "02:15", "Australia/Lord_Howe", "gap"],
	["2026-08-15", "09:30", "UTC", at("2026-08-15T09:30:00Z")],
	["2026-02-30", "09:30", "UTC", "date"],
	["2026-08-15", "9:30", "UTC", "time"],
	["2026-08-15", "09:30:00", "UTC", "time"],
	["2026-08-15", "24:00", "UTC", "time"],
	["1999-12-31", "23:59", "UTC", "range"],
];

test("a local date and time in a zone is one instant, the earlier of two, or none", () => {
	for (const [date, time, zone, expected] of CASES)
		expect([date, time, zone, resolveLocalInstant(date, time, zone)]).toEqual([
			date,
			time,
			zone,
			typeof expected === "number"
				? { ok: true, at: expected }
				: { ok: false, problem: expected },
		]);
});

test("the shape is checked before anything is parsed", () => {
	for (const date of [
		"2026-8-15",
		"20260815",
		"2026-13-01",
		"2026-00-10",
		"2026-04-31",
		"2026-08-15T09:30",
		"",
	])
		expect(resolveLocalInstant(date, "09:30", "UTC")).toEqual({
			ok: false,
			problem: "date",
		});
	for (const time of ["09:60", "9:3", "0930", "09.30", " 09:30", ""])
		expect(resolveLocalInstant("2026-08-15", time, "UTC")).toEqual({
			ok: false,
			problem: "time",
		});
	expect(resolveLocalInstant("2028-02-29", "12:00", "UTC")).toEqual({
		ok: true,
		at: at("2028-02-29T12:00:00Z"),
	});
	expect(resolveLocalInstant("2100-02-29", "12:00", "UTC")).toEqual({
		ok: false,
		problem: "date",
	});
	for (const zone of ["", "Mars/Olympus"])
		expect(resolveLocalInstant("2026-08-15", "09:30", zone)).toEqual({
			ok: false,
			problem: "zone",
		});
	expect(intlZone("")).toBe(false);
	expect(intlZone("Europe/Berlin")).toBe(true);
});

test("the instant stays within 2000 and 2100 by UTC, wherever the zone puts it", () => {
	expect(resolveLocalInstant("2000-01-01", "00:00", "UTC")).toEqual({
		ok: true,
		at: ONCE_MIN_AT,
	});
	expect(
		resolveLocalInstant("2000-01-01", "09:00", "Pacific/Kiritimati"),
	).toEqual({ ok: false, problem: "range" });
	expect(resolveLocalInstant("2100-01-01", "00:00", "UTC")).toEqual({
		ok: true,
		at: ONCE_MAX_AT,
	});
	expect(resolveLocalInstant("2100-01-01", "00:01", "UTC")).toEqual({
		ok: false,
		problem: "range",
	});
	expect(resolveLocalInstant("0099-01-01", "00:00", "UTC")).toEqual({
		ok: false,
		problem: "range",
	});
});

test("half-hour zones and the southern summer resolve like every other zone", () => {
	expect(resolveLocalInstant("2026-06-01", "12:00", "Asia/Kolkata")).toEqual({
		ok: true,
		at: at("2026-06-01T06:30:00Z"),
	});
	// Lord Howe goes back half an hour on 2026-04-05 at 02:00: 01:45 exists twice.
	expect(
		resolveLocalInstant("2026-04-05", "01:45", "Australia/Lord_Howe"),
	).toEqual({ ok: true, at: at("2026-04-04T14:45:00Z") });
	expect(
		resolveLocalInstant("2026-01-15", "12:00", "Australia/Sydney"),
	).toEqual({ ok: true, at: at("2026-01-15T01:00:00Z") });
});
