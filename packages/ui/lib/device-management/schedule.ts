import { CronExpressionParser } from "cron-parser";
import {
	type IScheduleConfig,
	projectScheduleConfig,
} from "../schedule-config";
import { intlZone, resolveLocalInstant } from "../schedule-instant";

/*
 * The schedule rule a device applies to a `cron` event before it runs it. The
 * device's check is the authority; this copy shows the reason before anything
 * is uploaded, so both sides only accept syntax they read the same way.
 */

/** `schedule_once` is only what an agent without one-time schedules answers; this check never gives it. */
export const SCHEDULE_PROBLEMS = [
	"schedule_missing",
	"schedule_once",
	"schedule_invalid",
	"schedule_too_often",
] as const;
export type ScheduleProblem = (typeof SCHEDULE_PROBLEMS)[number];

/** What this check found wrong with a schedule; the copy layer words it. */
export type ScheduleDetail =
	| { code: "both" }
	| { code: "length" }
	| { code: "fields"; count: number }
	| { code: "syntax"; field: string }
	| { code: "parser" }
	| { code: "zone"; zone: string }
	/** One-time: not `YYYY-MM-DD`, or no such calendar day. */
	| { code: "date"; date: string }
	/** One-time: not `HH:MM` from 00:00 to 23:59. */
	| { code: "time"; time: string }
	/** One-time: the clocks skip that local time in the zone. */
	| { code: "gap"; date: string; time: string; zone: string }
	/** One-time: before 2000 or after 2100. */
	| { code: "range"; date: string };

export interface DeviceSchedule {
	/** Single-spaced and upper-cased, as the device parses it. */
	expression: string;
	/** The saved zone, else UTC: a device never uses its own zone. */
	timezone: string;
	/** False when the event saves no zone. */
	zoneSet: boolean;
}

/** A one-time schedule a device can run. Whether its time has passed is state, never validity. */
export interface OnceSchedule {
	date: string;
	time: string;
	/** Unix seconds. */
	at: number;
	/** The saved zone, else UTC. */
	timezone: string;
	zoneSet: boolean;
}

export type DeviceScheduleResult =
	| { ok: true; schedule: DeviceSchedule; once?: undefined }
	| { ok: true; once: OnceSchedule; schedule?: undefined }
	| { ok: false; problem: ScheduleProblem; detail?: ScheduleDetail };

export const MAX_SCHEDULE_EXPRESSION = 128;

const PLAIN_FIELD = /^[0-9*,\-/]+$/;
const NAMED_FIELD = /^[0-9A-Za-z*,\-/]+$/;
const MONTH_NAMES = new Set([
	"JAN",
	"FEB",
	"MAR",
	"APR",
	"MAY",
	"JUN",
	"JUL",
	"AUG",
	"SEP",
	"OCT",
	"NOV",
	"DEC",
]);
const WEEKDAY_NAMES = new Set([
	"SUN",
	"MON",
	"TUE",
	"WED",
	"THU",
	"FRI",
	"SAT",
]);
const ZONE = /^[A-Za-z0-9_+\-/]{1,64}$/;

const invalid = (detail: ScheduleDetail): DeviceScheduleResult => ({
	ok: false,
	problem: "schedule_invalid",
	detail,
});

/** The first field a device does not read: only `*`, numbers, `,`, `-`, `/`, and names in the month and weekday fields. */
function unreadableField(fields: readonly string[]): string | undefined {
	const month = fields.length - 2;
	return fields.find((field, index) => {
		if (index < month) return !PLAIN_FIELD.test(field);
		if (!NAMED_FIELD.test(field)) return true;
		const names = index === month ? MONTH_NAMES : WEEKDAY_NAMES;
		return (field.match(/[A-Za-z]+/g) ?? []).some(
			(name) => !names.has(name.toUpperCase()),
		);
	});
}

/** The parser reads it and it has a time after `nowMs`; "30 February" parses on a device and never runs. */
function hasNextTime(expression: string, nowMs: number): boolean {
	try {
		CronExpressionParser.parse(expression, {
			tz: "UTC",
			currentDate: new Date(nowMs),
		}).next();
		return true;
	} catch {
		return false;
	}
}

/**
 * A zone name a device reads. `Intl` also takes offsets (`+02:00`) and names
 * in another case (`utc`, `europe/berlin`), which a device refuses.
 */
export function knownZone(zone: string): boolean {
	if (!ZONE.test(zone) || /^[+-]/.test(zone) || !intlZone(zone)) return false;
	const resolved = new Intl.DateTimeFormat("en-US", {
		timeZone: zone,
	}).resolvedOptions().timeZone;
	return resolved === zone || resolved.toLowerCase() !== zone.toLowerCase();
}

function onceSchedule(
	date: string,
	time: string,
	zone: string,
): DeviceScheduleResult {
	const timezone = zone || "UTC";
	const instant = resolveLocalInstant(
		date,
		time,
		knownZone(timezone) ? timezone : "",
	);
	if (instant.ok)
		return {
			ok: true,
			once: { date, time, at: instant.at, timezone, zoneSet: Boolean(zone) },
		};
	const detail: Record<typeof instant.problem, ScheduleDetail> = {
		date: { code: "date", date: date.slice(0, 32) },
		time: { code: "time", time: time.slice(0, 32) },
		zone: { code: "zone", zone: timezone.slice(0, 64) },
		gap: { code: "gap", date, time, zone: timezone },
		range: { code: "range", date: date.slice(0, 32) },
	};
	return invalid(detail[instant.problem]);
}

function checkExpression(
	text: string,
	nowMs: number,
): DeviceScheduleResult | string {
	const trimmed = text.trim();
	if (trimmed.length > MAX_SCHEDULE_EXPRESSION)
		return invalid({ code: "length" });
	const fields = trimmed.split(/\s+/);
	if (fields.length !== 5 && fields.length !== 6)
		return invalid({ code: "fields", count: fields.length });
	const unreadable = unreadableField(fields);
	if (unreadable !== undefined)
		return invalid({ code: "syntax", field: unreadable.slice(0, 32) });
	// With a seconds field a schedule runs at most once a minute only when that field is one number.
	if (fields.length === 6 && !/^\d+$/.test(fields[0]))
		return { ok: false, problem: "schedule_too_often" };
	const expression = fields.join(" ").toUpperCase();
	return hasNextTime(expression, nowMs)
		? expression
		: invalid({ code: "parser" });
}

/**
 * Whether a device can run this schedule, from the event's config (the raw
 * record or its schedule half). `nowMs` is the time "has a next run" is asked for.
 */
export function deviceSchedule(
	config: Record<string, unknown> | IScheduleConfig | null | undefined,
	nowMs: number = Date.now(),
): DeviceScheduleResult {
	const { expression, timezone, scheduled_for } = projectScheduleConfig(
		config as Record<string, unknown> | null | undefined,
	);
	const text = expression?.trim();
	if (scheduled_for)
		return text
			? invalid({ code: "both" })
			: onceSchedule(
					scheduled_for.date ?? "",
					scheduled_for.time ?? "",
					timezone?.trim() ?? "",
				);
	if (!text) return { ok: false, problem: "schedule_missing" };
	const checked = checkExpression(text, nowMs);
	if (typeof checked !== "string") return checked;
	const zone = timezone?.trim();
	if (zone && !knownZone(zone))
		return invalid({ code: "zone", zone: zone.slice(0, 64) });
	return {
		ok: true,
		schedule: {
			expression: checked,
			timezone: zone || "UTC",
			zoneSet: Boolean(zone),
		},
	};
}

/** The next `count` times after `fromMs` in unix milliseconds; fewer when the schedule can't be read or ends. */
export function nextRuns(
	schedule: Pick<DeviceSchedule, "expression" | "timezone">,
	fromMs: number,
	count: number,
): number[] {
	const times: number[] = [];
	try {
		const iterator = CronExpressionParser.parse(schedule.expression, {
			tz: schedule.timezone,
			currentDate: new Date(fromMs),
		});
		for (let index = 0; index < count; index++)
			times.push(iterator.next().getTime());
	} catch {
		// What was found so far is the answer.
	}
	return times;
}

/** Whether a one-time schedule's instant is still after `nowMs`. */
export function onceAhead(
	once: Pick<OnceSchedule, "at">,
	nowMs: number,
): boolean {
	return once.at * 1000 > nowMs;
}
