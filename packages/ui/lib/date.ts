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

/** Trailing words that make a column an instant rather than a measurement. */
const TEMPORAL_SUFFIXES = new Set([
	"at",
	"on",
	"ts",
	"time",
	"timestamp",
	"datetime",
	"date",
	"since",
	"until",
	"created",
	"updated",
	"modified",
	"deleted",
	"expired",
	"expires",
	"published",
	"scheduled",
]);
/** Leading words that do the same, for `dateOfBirth` and `time_received`. */
const TEMPORAL_PREFIXES = new Set([
	"date",
	"time",
	"timestamp",
	"datetime",
	"dob",
]);

function nameSegments(name: string): string[] {
	return name
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.split(/[^a-zA-Z0-9]+/)
		.filter(Boolean)
		.map((segment) => segment.toLowerCase());
}

/**
 * Whether a column name promises an instant.
 *
 * Deliberately anchored to the first and last word rather than matching anywhere:
 * a substring rule turns `duration`, `rating` and `estimate` into timestamps, and
 * a number rendered as a date is a worse failure than a date rendered as a number.
 */
export function looksLikeTemporalName(name: string): boolean {
	const segments = nameSegments(name);
	if (segments.length === 0) return false;
	return (
		TEMPORAL_SUFFIXES.has(segments[segments.length - 1]) ||
		TEMPORAL_PREFIXES.has(segments[0])
	);
}

/** Calendar window a bare epoch number has to land in to be believed. */
const MIN_PLAUSIBLE_YEAR = 1990;
const MAX_PLAUSIBLE_YEAR = 2100;

/**
 * Reads a value whose column was never *declared* temporal — the common case,
 * because backends store instants as plain integers (`created_at` as epoch
 * millis) and the schema then says nothing but `Int64`.
 *
 * A number is only believed when the name promises an instant *and* the result
 * lands in a plausible calendar window; text still goes through the ordinary
 * parser, where an ISO string speaks for itself.
 */
export function inferTemporalValue(name: string, value: unknown): Date | null {
	if (typeof value === "number" || typeof value === "bigint") {
		if (!looksLikeTemporalName(name)) return null;
		const parsed = parseTemporalValue(value);
		if (!parsed) return null;
		const year = parsed.getFullYear();
		return year >= MIN_PLAUSIBLE_YEAR && year <= MAX_PLAUSIBLE_YEAR
			? parsed
			: null;
	}
	if (value instanceof Date) return parseDateValue(value);
	if (typeof value !== "string") return null;

	// A bare numeric string is the integer case wearing quotes, and reaches the
	// string parser as an invalid date rather than as the epoch it holds.
	const trimmed = value.trim();
	if (/^-?\d+$/.test(trimmed)) {
		return inferTemporalValue(name, Number(trimmed));
	}
	return parseDateValue(trimmed);
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
