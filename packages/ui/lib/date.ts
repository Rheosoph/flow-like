import { format, formatDistanceToNow } from "date-fns";
import type { IDate } from "../types";

export type DateValue = Date | IDate | number | string | null | undefined;

function parseChronoDateString(value: string): Date | null {
	const trimmed = value.trim();
	const chronoMatch = trimmed.match(
		/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?(?:\s*(UTC|Z))?$/,
	);

	if (!chronoMatch) {
		return null;
	}

	const [, datePart, timePart, fractionalPart, zonePart] = chronoMatch;
	const milliseconds = fractionalPart
		? `.${fractionalPart.padEnd(3, "0").slice(0, 3)}`
		: "";
	const normalized = `${datePart}T${timePart}${milliseconds}${zonePart ? "Z" : "Z"}`;
	const parsed = new Date(normalized);

	return Number.isNaN(parsed.getTime()) ? null : parsed;
}

function isSystemDate(value: DateValue): value is IDate {
	return (
		typeof value === "object" &&
		value !== null &&
		"secs_since_epoch" in value &&
		typeof value.secs_since_epoch === "number"
	);
}

export function parseDateValue(value: DateValue): Date | null {
	if (value == null) {
		return null;
	}

	if (value instanceof Date) {
		return Number.isNaN(value.getTime()) ? null : value;
	}

	if (isSystemDate(value)) {
		const milliseconds =
			value.secs_since_epoch * 1000 +
			Math.floor((value.nanos_since_epoch ?? 0) / 1_000_000);
		const parsed = new Date(milliseconds);
		return Number.isNaN(parsed.getTime()) ? null : parsed;
	}

	if (typeof value === "string") {
		// Backend timestamps are naive UTC (chrono NaiveDateTime, no zone suffix).
		// new Date() parses an offset-less date-time as LOCAL time, so resolve the
		// UTC-aware form first and only fall back for formats it doesn't recognize.
		const chrono = parseChronoDateString(value);
		if (chrono) {
			return chrono;
		}

		const parsed = new Date(value);
		return Number.isNaN(parsed.getTime()) ? null : parsed;
	}

	if (typeof value === "number") {
		const parsed = new Date(value);
		return Number.isNaN(parsed.getTime()) ? null : parsed;
	}

	return null;
}

export function formatRelativeDateValue(
	dateInput: DateValue,
	fallback = "Unknown",
) {
	const parsed = parseDateValue(dateInput);
	return parsed
		? formatDistanceToNow(parsed, {
				addSuffix: true,
			})
		: fallback;
}

export function formatAbsoluteDateValue(
	dateInput: DateValue,
	fallback = "Unknown",
	pattern = "PPp",
) {
	const parsed = parseDateValue(dateInput);
	return parsed ? format(parsed, pattern) : fallback;
}

/** Days in the average month and year, so the ladder rolls over where a reader expects. */
const RELATIVE_DIVISIONS: readonly {
	amount: number;
	unit: Intl.RelativeTimeFormatUnit;
}[] = [
	{ amount: 60, unit: "second" },
	{ amount: 60, unit: "minute" },
	{ amount: 24, unit: "hour" },
	{ amount: 7, unit: "day" },
	{ amount: 4.34524, unit: "week" },
	{ amount: 12, unit: "month" },
	{ amount: Number.POSITIVE_INFINITY, unit: "year" },
];

/**
 * "2 days ago" in the viewer's locale. Walks the whole unit ladder, so a value
 * years old reads as years rather than as a four-digit day count.
 */
export function formatRelativeTime(
	dateInput: DateValue,
	style: Intl.RelativeTimeFormatStyle = "long",
	fallback = "Invalid date",
) {
	const parsed = parseDateValue(dateInput);
	const targetTimeMs = parsed?.getTime() ?? Number.NaN;

	if (Number.isNaN(targetTimeMs)) {
		return fallback;
	}

	const formatter = new Intl.RelativeTimeFormat(undefined, {
		numeric: "auto",
		style: style,
	});

	let duration = (targetTimeMs - Date.now()) / 1000;
	for (const division of RELATIVE_DIVISIONS) {
		if (Math.abs(duration) < division.amount) {
			return formatter.format(Math.round(duration), division.unit);
		}
		duration /= division.amount;
	}
	return formatter.format(Math.round(duration), "year");
}

/** The unambiguous reading of a timestamp, for tooltips behind a relative label. */
export function formatAbsoluteDateTime(dateInput: DateValue, fallback = "") {
	const parsed = parseDateValue(dateInput);
	if (!parsed) return fallback;
	return parsed.toLocaleString(undefined, {
		dateStyle: "full",
		timeStyle: "medium",
	});
}

/** Below this a number is a day count (chrono/Arrow Date32), not an instant. */
const MAX_EPOCH_DAYS = 100_000;
const MAX_EPOCH_SECONDS = 1e11;
const MAX_EPOCH_MILLIS = 1e14;
const MAX_EPOCH_MICROS = 1e17;

function parseEpochNumber(value: number): Date | null {
	if (!Number.isFinite(value)) return null;

	const magnitude = Math.abs(value);
	const milliseconds =
		magnitude < MAX_EPOCH_DAYS
			? value * 86_400_000
			: magnitude < MAX_EPOCH_SECONDS
				? value * 1000
				: magnitude < MAX_EPOCH_MILLIS
					? value
					: magnitude < MAX_EPOCH_MICROS
						? value / 1000
						: value / 1_000_000;

	const parsed = new Date(milliseconds);
	return Number.isNaN(parsed.getTime()) ? null : parsed;
}

/**
 * Parses a value a column has already been *declared* temporal — only then is a
 * bare number an instant rather than a quantity, and the magnitude decides which
 * epoch unit a backend meant (Arrow ships days, seconds, millis, micros and nanos).
 *
 * Numeric strings stay with the string parser: `"2026"` is a year to every reader
 * and a day count to this ladder.
 */
export function parseTemporalValue(value: unknown): Date | null {
	if (typeof value === "bigint") return parseEpochNumber(Number(value));
	if (typeof value === "number") return parseEpochNumber(value);
	if (
		typeof value === "string" ||
		typeof value === "object" ||
		value === undefined
	) {
		return parseDateValue(value as DateValue);
	}
	return null;
}

export function parseTimespan(start: IDate, end: IDate) {
	if (start.nanos_since_epoch > end.nanos_since_epoch) {
		const old_end = end;
		end = start;
		start = old_end;
	}

	const diff = end.nanos_since_epoch - start.nanos_since_epoch;
	const μs = diff / 1000;

	if (μs < 1000) return `${μs.toFixed(2)}μs`;
	const ms = μs / 1000;
	if (ms < 1000) return `${ms.toFixed(2)}ms`;
	const s = ms / 1000;
	if (s < 60) return `${s.toFixed(2)}s`;
	const m = s / 60;
	if (m < 60) return `${m.toFixed(2)}m`;
	const h = m / 60;
	if (h < 24) return `${h.toFixed(2)}h`;
	const d = h / 24;
	return `${d.toFixed(2)}d`;
}

export function formatDuration(μs: number) {
	if (μs < 1000) return `${μs.toFixed(2)} μs`;
	const ms = μs / 1000;
	if (ms < 1000) return `${ms.toFixed(2)} ms`;
	const s = ms / 1000;
	if (s < 60) return `${s.toFixed(2)} s`;
	const m = s / 60;
	if (m < 60) return `${m.toFixed(2)} m`;
	const h = m / 60;
	if (h < 24) return `${h.toFixed(2)} h`;
	const d = h / 24;
	return `${d.toFixed(2)}d`;
}
