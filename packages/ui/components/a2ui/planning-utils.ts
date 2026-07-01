// Shared date / geometry helpers for the interactive Calendar and Gantt
// components. Kept free of React so the math can be unit-tested directly.

import {
	addDays,
	addMinutes,
	differenceInCalendarDays,
	differenceInMinutes,
	eachDayOfInterval,
	endOfDay,
	endOfMonth,
	endOfWeek,
	parseISO,
	startOfMonth,
	startOfWeek,
} from "date-fns";
import type { CalendarEvent, GanttTask } from "./types";

export type WeekStartsOn = 0 | 1 | 2 | 3 | 4 | 5 | 6;

/** Parse an ISO/loose date string (or epoch ms) into a Date, never NaN. */
export function toDate(value: string | number | Date | undefined | null): Date {
	if (value instanceof Date) return value;
	if (typeof value === "number") return new Date(value);
	if (typeof value === "string" && value) {
		const iso = parseISO(value);
		if (!Number.isNaN(iso.getTime())) return iso;
		const loose = new Date(value);
		if (!Number.isNaN(loose.getTime())) return loose;
	}
	return new Date();
}

function coerceArray(raw: unknown): unknown[] {
	if (Array.isArray(raw)) return raw;
	if (typeof raw === "string" && raw.trim()) {
		try {
			const parsed = JSON.parse(raw);
			if (Array.isArray(parsed)) return parsed;
		} catch {}
	}
	return [];
}

function toBool(value: unknown): boolean | undefined {
	if (typeof value === "boolean") return value;
	if (value === "true") return true;
	if (value === "false") return false;
	return undefined;
}

/** Coerce arbitrary flow output into well-formed CalendarEvent objects. */
export function normalizeCalendarEvents(raw: unknown): CalendarEvent[] {
	return coerceArray(raw)
		.filter((e): e is Record<string, unknown> => !!e && typeof e === "object")
		.map((e, i) => ({
			id: String(e.id ?? `event-${i}`),
			title: String(e.title ?? e.name ?? "Untitled"),
			start: String(e.start ?? e.date ?? e.startDate ?? ""),
			end: e.end != null ? String(e.end) : undefined,
			allDay: toBool(e.allDay ?? e.all_day),
			color: e.color != null ? String(e.color) : undefined,
			description: e.description != null ? String(e.description) : undefined,
			location: e.location != null ? String(e.location) : undefined,
			calendarId: e.calendarId != null ? String(e.calendarId) : undefined,
			editable: toBool(e.editable),
			metadata: e.metadata,
		}))
		.filter((e) => e.start);
}

/** Coerce arbitrary flow output into well-formed GanttTask objects. */
export function normalizeGanttTasks(raw: unknown): GanttTask[] {
	return coerceArray(raw)
		.filter((t): t is Record<string, unknown> => !!t && typeof t === "object")
		.map((t, i) => ({
			id: String(t.id ?? `task-${i}`),
			name: String(t.name ?? t.title ?? "Untitled"),
			start: String(t.start ?? t.startDate ?? ""),
			end: String(t.end ?? t.endDate ?? t.start ?? ""),
			progress:
				t.progress != null && !Number.isNaN(Number(t.progress))
					? Number(t.progress)
					: undefined,
			dependencies: Array.isArray(t.dependencies)
				? t.dependencies.map(String)
				: undefined,
			parent: t.parent != null ? String(t.parent) : undefined,
			color: t.color != null ? String(t.color) : undefined,
			assignee: t.assignee != null ? String(t.assignee) : undefined,
			milestone: toBool(t.milestone),
			collapsed: toBool(t.collapsed),
			metadata: t.metadata,
		}))
		.filter((t) => t.start && t.end);
}

/** Effective end of a calendar event (honours allDay / default 1h duration). */
export function eventEnd(ev: CalendarEvent): Date {
	const start = toDate(ev.start);
	if (ev.end) return toDate(ev.end);
	return ev.allDay ? endOfDay(start) : addMinutes(start, 60);
}

/** Duration of a calendar event in minutes (min 15). */
export function eventDurationMinutes(ev: CalendarEvent): number {
	return Math.max(15, differenceInMinutes(eventEnd(ev), toDate(ev.start)));
}

/** Weeks (rows of 7 days) covering the month that `date` falls in. */
export function getMonthWeeks(
	date: Date,
	weekStartsOn: WeekStartsOn,
): Date[][] {
	const start = startOfWeek(startOfMonth(date), { weekStartsOn });
	const end = endOfWeek(endOfMonth(date), { weekStartsOn });
	const days = eachDayOfInterval({ start, end });
	const weeks: Date[][] = [];
	for (let i = 0; i < days.length; i += 7) weeks.push(days.slice(i, i + 7));
	return weeks;
}

/** The 7 days of the week that `date` falls in. */
export function getWeekDays(date: Date, weekStartsOn: WeekStartsOn): Date[] {
	const start = startOfWeek(date, { weekStartsOn });
	return Array.from({ length: 7 }, (_, i) => addDays(start, i));
}

/** Parse a "HH:MM" string to minutes-from-midnight. Defaults on bad input. */
export function parseTimeToMinutes(
	value: string | undefined,
	fallback: number,
): number {
	if (!value) return fallback;
	const [h, m] = value.split(":");
	const hours = Number(h);
	const minutes = Number(m);
	if (Number.isNaN(hours)) return fallback;
	return hours * 60 + (Number.isNaN(minutes) ? 0 : minutes);
}

export interface GanttRange {
	start: Date;
	end: Date;
	totalDays: number;
}

/** Inclusive day-range spanning all tasks, padded by `padDays` on each side. */
export function ganttRange(tasks: GanttTask[], padDays = 2): GanttRange {
	if (tasks.length === 0) {
		const start = new Date();
		const end = addDays(start, 30);
		return { start, end, totalDays: differenceInCalendarDays(end, start) + 1 };
	}
	let min = toDate(tasks[0].start);
	let max = toDate(tasks[0].end);
	for (const t of tasks) {
		const s = toDate(t.start);
		const e = toDate(t.end);
		if (s < min) min = s;
		if (e > max) max = e;
	}
	const start = addDays(min, -padDays);
	const end = addDays(max, padDays);
	return { start, end, totalDays: differenceInCalendarDays(end, start) + 1 };
}

/** Left offset + width (in day units) of a task bar within a Gantt range. */
export function taskBarDays(
	task: GanttTask,
	range: GanttRange,
): { offsetDays: number; spanDays: number } {
	const offsetDays = differenceInCalendarDays(toDate(task.start), range.start);
	const spanDays = Math.max(
		1,
		differenceInCalendarDays(toDate(task.end), toDate(task.start)) + 1,
	);
	return { offsetDays, spanDays };
}
