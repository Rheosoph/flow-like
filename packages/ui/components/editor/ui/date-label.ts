import {
	formatDateValue,
	getDateDisplayLabel,
	normalizeDateValue,
	parseCanonicalDateValue,
} from "@platejs/date";
import type { TDateElement } from "platejs";

type StoredDate = Pick<TDateElement, "date" | "rawDate">;

/**
 * Calendar day of a date node as `YYYY-MM-DD`. Reads the canonical value,
 * `Date#toDateString()` output and anything else `Date` parses, in local time.
 */
export function getDateElementDay({
	date,
	rawDate,
}: StoredDate): string | undefined {
	const value: unknown = date || rawDate;
	if (!value) return;
	if (typeof value === "string") {
		const normalized = normalizeDateValue(value);
		if (normalized.date) return normalized.date;
	}
	const parsed = new Date(value as string | number);
	return Number.isNaN(parsed.getTime()) ? undefined : formatDateValue(parsed);
}

export function getDateElementDate(element: StoredDate): Date | undefined {
	const day = getDateElementDay(element);
	return day ? parseCanonicalDateValue(day) : undefined;
}

/** "Today", "Yesterday", "Tomorrow" or the long local date; unreadable values as stored. */
export function getDateElementLabel(
	element: StoredDate,
	now?: Date,
): string | undefined {
	const day = getDateElementDay(element);
	if (day) return getDateDisplayLabel({ date: day, now });
	const value: unknown = element.date || element.rawDate;
	return value ? String(value) : undefined;
}
