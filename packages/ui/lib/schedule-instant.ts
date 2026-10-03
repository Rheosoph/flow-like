/*
 * The instant of a one-time schedule: a local date and time in a zone. A
 * device applies the same rule, so both sides read one instant or refuse.
 */

export type InstantProblem = "date" | "time" | "zone" | "gap" | "range";

export type LocalInstant =
	| { ok: true; at: number }
	| { ok: false; problem: InstantProblem };

/** Unix seconds: 2000-01-01T00:00:00Z and 2100-01-01T00:00:00Z, both allowed. */
export const ONCE_MIN_AT = 946_684_800;
export const ONCE_MAX_AT = 4_102_444_800;

const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const TIME = /^(\d{2}):(\d{2})$/;
const DAY_MS = 86_400_000;
const MONTH_DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const within = (value: number, min: number, max: number) =>
	value >= min && value <= max;

function daysIn(year: number, month: number) {
	const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
	return month === 2 && leap ? 29 : MONTH_DAYS[month - 1];
}

const formats = new Map<string, Intl.DateTimeFormat>();
function zoneFormat(zone: string) {
	if (!zone) return undefined;
	const known = formats.get(zone);
	if (known) return known;
	try {
		const created = new Intl.DateTimeFormat("en-US", {
			timeZone: zone,
			year: "numeric",
			month: "2-digit",
			day: "2-digit",
			hour: "2-digit",
			minute: "2-digit",
			hourCycle: "h23",
		});
		formats.set(zone, created);
		return created;
	} catch {
		return undefined;
	}
}

/** The wall clock of `ms` in the zone, read as if it were UTC. */
function wallClock(format: Intl.DateTimeFormat, ms: number) {
	const parts: Record<string, string> = {};
	for (const part of format.formatToParts(ms)) parts[part.type] = part.value;
	return Date.UTC(
		Number(parts.year),
		Number(parts.month) - 1,
		Number(parts.day),
		Number(parts.hour),
		Number(parts.minute),
	);
}

/** A zone `Intl` knows; the empty name is none. */
export function intlZone(zone: string) {
	return zoneFormat(zone) !== undefined;
}

/** `[year, month, day]` of a real calendar day written `YYYY-MM-DD`. */
function calendarDay(date: string) {
	const parts = DATE.exec(date)?.slice(1).map(Number);
	if (!parts) return undefined;
	const [year, month, day] = parts;
	const real = month >= 1 && month <= 12 && day >= 1;
	return real && day <= daysIn(year, month) ? parts : undefined;
}

/** `[hour, minute]` of a time written `HH:MM`. */
function clockTime(time: string) {
	const parts = TIME.exec(time)?.slice(1).map(Number);
	return parts && parts[0] <= 23 && parts[1] <= 59 ? parts : undefined;
}

/** The earliest instant whose wall clock in the zone reads `wanted`. */
function earliestInstant(format: Intl.DateTimeFormat, wanted: number) {
	const offsets = new Set(
		[-2, -1, 0, 1, 2].map((shift) => {
			const at = wanted + shift * DAY_MS;
			return wallClock(format, at) - at;
		}),
	);
	return [...offsets]
		.map((offset) => wanted - offset)
		.filter((at) => wallClock(format, at) === wanted)
		.sort((a, b) => a - b)
		.at(0);
}

/**
 * The instant of `date` (`YYYY-MM-DD`) at `time` (`HH:MM`) in `zone`, in unix
 * seconds. A time that exists twice resolves to the earlier one; a time the
 * clocks skip has none. Never compared with a clock.
 */
export function resolveLocalInstant(
	date: string,
	time: string,
	zone: string,
): LocalInstant {
	const day = calendarDay(date);
	if (!day) return { ok: false, problem: "date" };
	const clock = clockTime(time);
	if (!clock) return { ok: false, problem: "time" };
	const format = zoneFormat(zone);
	if (!format) return { ok: false, problem: "zone" };
	const [year, month, dayOfMonth] = day;
	// Far years are refused before `Date` maps them (years below 100 are read as 19xx).
	if (!within(year, 1990, 2110)) return { ok: false, problem: "range" };
	const first = earliestInstant(
		format,
		Date.UTC(year, month - 1, dayOfMonth, clock[0], clock[1]),
	);
	if (first === undefined) return { ok: false, problem: "gap" };
	const at = Math.floor(first / 1000);
	return within(at, ONCE_MIN_AT, ONCE_MAX_AT)
		? { ok: true, at }
		: { ok: false, problem: "range" };
}
