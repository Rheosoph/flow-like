import { CronExpressionParser } from "cron-parser";
import { addDays } from "date-fns";
import type {
	IBotTask,
	IBotTaskEvent,
	ICalendarEntry,
	ICalendarEntryWake,
	ICalendarRange,
	ICreateBotTaskRequest,
	IEventWake,
	IExternalEventRef,
	IUpdateBotTaskRequest,
} from "../../state/backend-state/bot-state";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
import {
	eventEnd,
	toDate,
	toDateInput,
	toDateTimeLocalInput,
} from "../a2ui/planning-utils";
import type { CalendarEvent } from "../a2ui/types";
import {
	DAY_MS,
	calendarInstant,
	parseInstant,
	wholeDayZone,
} from "./bot-memory-model";
import { scheduledTasks } from "./bot-task-model";

export const CALENDAR_LAYERS = [
	"events",
	"external",
	"tasks",
	"memories",
] as const;
export type CalendarLayer = (typeof CALENDAR_LAYERS)[number];

/** Design tokens, so the calendar's chips follow the theme. */
export const CALENDAR_LAYER_COLORS: Readonly<Record<CalendarLayer, string>> = {
	events: "var(--primary)",
	external: "var(--info-solid)",
	tasks: "var(--warning-solid)",
	memories: "var(--locked-solid)",
};

export const WAKE_OFFSET_CHOICES = [5, 10, 15, 30, 60, 120, 1440] as const;
export const MAX_WAKE_OFFSETS = 4;
export const MAX_WAKE_PATTERNS = 20;
export const MAX_WAKE_PATTERN_CHARS = 80;
/** The longest window one calendar listing covers. */
export const MAX_CALENDAR_RANGE_DAYS = 92;
/** A remembered span longer than this shows where it starts and ends instead of covering every day. */
const MAX_MEMORY_SPAN_DAYS = 31;
/** How long a moment without an end looks in the calendar. */
const POINT_MINUTES = 30;
const MAX_TASK_OCCURRENCES = 100;

/** What a calendar chip stands for. */
export type CalendarSubject =
	| { layer: "events"; entry: ICalendarEntry }
	| { layer: "external"; entry: ICalendarEntry }
	| { layer: "tasks"; task: IBotTask; at: string }
	| { layer: "memories"; fact: BotFact };

export interface CalendarItem {
	event: CalendarEvent;
	subject: CalendarSubject;
}

type Span = Pick<CalendarEvent, "start" | "end" | "allDay">;

function isoOf(time: number): string {
	return new Date(time).toISOString();
}

function timedSpan(start: number, end: number | null): Span {
	return {
		start: isoOf(start),
		end: isoOf(
			end !== null && end > start ? end : start + POINT_MINUTES * 60_000,
		),
		allDay: false,
	};
}

/** Whole days from `start` to `end`; a whole-day `end` that is exclusive names the day after the last. */
function allDaySpan(
	start: number,
	end: number | null,
	exclusiveEnd: boolean,
): Span {
	const first = calendarInstant(start);
	let last = end === null ? first : calendarInstant(end);
	if (
		exclusiveEnd &&
		end !== null &&
		wholeDayZone(end) !== null &&
		last > first
	)
		last = addDays(last, -1).getTime();
	return {
		start: toDateInput(new Date(first)),
		end: toDateInput(new Date(Math.max(first, last))),
		allDay: true,
	};
}

/** A date said without a time of day is a whole day; anything else a short moment. */
function pointSpan(time: number): Span {
	return wholeDayZone(time) === null
		? timedSpan(time, null)
		: allDaySpan(time, null, false);
}

function entryItem(
	entry: ICalendarEntry,
	editableOwn: boolean,
): CalendarItem | null {
	const start = parseInstant(entry.starts_at);
	if (start === null) return null;
	const layer = entry.source === "external" ? "external" : "events";
	const end = parseInstant(entry.ends_at);
	return {
		event: {
			id: `${layer}:${entry.id}:${entry.starts_at}`,
			title: entry.title,
			...(entry.all_day ? allDaySpan(start, end, true) : timedSpan(start, end)),
			location: entry.location ?? undefined,
			description: entry.notes ?? undefined,
			color: CALENDAR_LAYER_COLORS[layer],
			calendarId: layer,
			editable:
				layer === "events" &&
				editableOwn &&
				!entry.recurring &&
				Boolean(entry.task_id),
		},
		subject: { layer, entry },
	};
}

/** A once task at its time; an active cron task's runs from its next run on. */
export function taskOccurrences(
	task: IBotTask,
	from: number,
	to: number,
): string[] {
	if (task.kind === "once") {
		const at = parseInstant(task.at);
		return at !== null && at >= from && at < to ? [isoOf(at)] : [];
	}
	if (task.kind !== "cron" || !task.enabled) return [];
	const next = parseInstant(task.next_run_at);
	if (next === null || next >= to) return [];
	const first = Math.max(from, next);
	try {
		const runs = CronExpressionParser.parse(task.cron ?? "", {
			currentDate: new Date(first - 1),
			tz: task.tz || "UTC",
		});
		const times: string[] = [];
		while (times.length < MAX_TASK_OCCURRENCES) {
			const run = runs.next().toDate().getTime();
			if (run >= to) break;
			times.push(isoOf(run));
		}
		return times;
	} catch {
		return next >= from ? [isoOf(next)] : [];
	}
}

function taskItems(task: IBotTask, from: number, to: number): CalendarItem[] {
	return taskOccurrences(task, from, to).map((at) => ({
		event: {
			id: `tasks:${task.id}:${at}`,
			title: task.title,
			...timedSpan(Date.parse(at), null),
			color: CALENDAR_LAYER_COLORS.tasks,
			calendarId: "tasks",
			editable: false,
		},
		subject: { layer: "tasks", task, at },
	}));
}

function memoryItem(fact: BotFact, span: Span, edge = ""): CalendarItem {
	return {
		event: {
			id: `memories:${fact.id}${edge}`,
			title: fact.text,
			...span,
			color: CALENDAR_LAYER_COLORS.memories,
			calendarId: "memories",
			editable: false,
		},
		subject: { layer: "memories", fact },
	};
}

/**
 * A remembered date: its `time` as a day or a moment, otherwise its validity as a span. A span
 * longer than a month shows its start and its end. Superseded and expired facts stay out.
 */
export function memoryItems(fact: BotFact, now: number): CalendarItem[] {
	if (fact.superseded_by) return [];
	const expires = parseInstant(fact.expires_at);
	if (expires !== null && expires <= now) return [];
	const time = parseInstant(fact.time);
	if (time !== null) return [memoryItem(fact, pointSpan(time))];
	const from = parseInstant(fact.valid_from);
	const to = parseInstant(fact.valid_to);
	if (
		from !== null &&
		to !== null &&
		to >= from &&
		to - from <= MAX_MEMORY_SPAN_DAYS * DAY_MS
	) {
		const wholeDays = wholeDayZone(from) !== null && wholeDayZone(to) !== null;
		return [
			memoryItem(
				fact,
				wholeDays ? allDaySpan(from, to, false) : timedSpan(from, to),
			),
		];
	}
	return [
		...(from === null ? [] : [memoryItem(fact, pointSpan(from), ":from")]),
		...(to === null ? [] : [memoryItem(fact, pointSpan(to), ":to")]),
	];
}

export interface CalendarInputs {
	range?: ICalendarRange;
	tasks?: readonly IBotTask[];
	facts?: readonly BotFact[];
}

export interface CalendarWindow {
	from: number;
	to: number;
}

function overlaps(event: CalendarEvent, window: CalendarWindow): boolean {
	return (
		toDate(event.start).getTime() < window.to &&
		eventEnd(event).getTime() >= window.from
	);
}

/** Everything the shown layers hold in the window, as calendar chips. */
export function calendarItems(
	inputs: CalendarInputs,
	layers: ReadonlySet<CalendarLayer>,
	window: CalendarWindow,
	now: number,
	editableOwn: boolean,
): CalendarItem[] {
	const items: CalendarItem[] = [];
	for (const entry of inputs.range?.entries ?? []) {
		const item = entryItem(entry, editableOwn);
		if (item && layers.has(item.subject.layer)) items.push(item);
	}
	if (layers.has("tasks")) {
		for (const task of scheduledTasks(inputs.tasks ?? []))
			items.push(...taskItems(task, window.from, window.to));
	}
	if (layers.has("memories")) {
		for (const fact of inputs.facts ?? [])
			items.push(...memoryItems(fact, now));
	}
	return items.filter((item) => overlaps(item.event, window));
}

/** The listing window for the days on screen, as the hosts take it. */
export function calendarWindow(range: { from: Date; to: Date }): {
	from: string;
	to: string;
} {
	const from = range.from.getTime();
	const to = Math.min(
		Math.max(range.to.getTime(), from + 1),
		from + MAX_CALENDAR_RANGE_DAYS * DAY_MS,
	);
	return { from: isoOf(from), to: isoOf(to) };
}

function unhex(text: string): string | null {
	if (!text || text.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(text)) return null;
	const bytes = new Uint8Array(text.length / 2);
	for (let index = 0; index < bytes.length; index++)
		bytes[index] = Number.parseInt(text.slice(index * 2, index * 2 + 2), 16);
	try {
		return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
	} catch {
		return null;
	}
}

/** The connected-calendar event an entry id names (`ext.<provider>.<connector>.<hex>.<hex>`). */
export function externalRefOf(id: string): IExternalEventRef | null {
	const parts = id.trim().split(".");
	if (parts.length !== 5 || parts[0] !== "ext") return null;
	const [, provider, connectorId, calendarHex, eventHex] = parts;
	const calendarId = unhex(calendarHex);
	const eventId = unhex(eventHex);
	if (!provider || !connectorId || calendarId === null || eventId === null)
		return null;
	return {
		connector_id: connectorId,
		provider,
		calendar_id: calendarId,
		event_id: eventId,
	};
}

/** `default` follows the bot's wake-up settings. */
export type WakeChoice = "default" | "on" | "off";

export function wakeChoiceOf(wake?: IEventWake | null): WakeChoice {
	if (!wake) return "default";
	return wake.enabled ? "on" : "off";
}

function sortedOffsets(offsets: readonly number[]): number[] {
	return [...new Set(offsets)].sort((a, b) => b - a);
}

/** The event's own wake-up for a choice; none follows the bot. */
export function eventWakeOf(
	choice: WakeChoice,
	offsets: readonly number[],
): IEventWake | undefined {
	if (choice === "default") return undefined;
	if (choice === "off") return { enabled: false, offsets_minutes: [] };
	return { enabled: true, offsets_minutes: sortedOffsets(offsets) };
}

/** Adds or removes one offset; at most four, largest first. */
export function toggleWakeOffset(
	offsets: readonly number[],
	minutes: number,
): number[] {
	if (offsets.includes(minutes))
		return offsets.filter((offset) => offset !== minutes);
	if (offsets.length >= MAX_WAKE_OFFSETS) return [...offsets];
	return sortedOffsets([...offsets, minutes]);
}

/** "15 min", "2 hr", "1 day" in the viewer's language. */
export function formatWakeOffset(minutes: number, locale?: string): string {
	const [value, unit] =
		minutes > 0 && minutes % 1440 === 0
			? [minutes / 1440, "day"]
			: minutes > 0 && minutes % 60 === 0
				? [minutes / 60, "hour"]
				: [minutes, "minute"];
	return new Intl.NumberFormat(locale, {
		style: "unit",
		unit,
		unitDisplay: "short",
	}).format(value);
}

/** "15 min, 1 day" — the offsets an entry wakes the bot at. */
export function formatWakeOffsets(
	wake: Pick<ICalendarEntryWake, "offsets_minutes">,
	locale?: string,
): string {
	return sortedOffsets(wake.offsets_minutes)
		.reverse()
		.map((minutes) => formatWakeOffset(minutes, locale))
		.join(", ");
}

/** The whole event with a new own wake-up; none clears it, and the host works out the offsets. */
export function eventWithWake(
	event: IBotTaskEvent,
	wake: IEventWake | undefined,
): IBotTaskEvent {
	const { wake: _previous, wake_offsets: _resolved, ...rest } = event;
	return wake ? { ...rest, wake } : rest;
}

/** Keeps wake-ups for a connected-calendar event: its row is created or found by its reference. */
export function externalWakeRequest(
	entry: ICalendarEntry,
	wake: IEventWake | undefined,
	tz: string,
): ICreateBotTaskRequest | null {
	const external = externalRefOf(entry.id);
	if (!external) return null;
	return {
		kind: "event",
		title: entry.title,
		instruction: "",
		tz,
		event: {
			starts_at: entry.starts_at,
			ends_at: entry.ends_at ?? null,
			all_day: Boolean(entry.all_day),
			location: entry.location ?? null,
			link: entry.link ?? null,
			external,
			...(wake ? { wake } : {}),
		},
	};
}

export const REPEAT_CHOICES = [
	"none",
	"daily",
	"weekdays",
	"weekly",
	"monthly",
] as const;
/** `custom` keeps a cron the presets don't describe. */
export type RepeatChoice = (typeof REPEAT_CHOICES)[number] | "custom";

/** The cron that repeats an event starting at `start`, in the viewer's zone. */
export function repeatCron(
	choice: Exclude<RepeatChoice, "none" | "custom">,
	start: Date,
): string {
	const time = `${start.getMinutes()} ${start.getHours()}`;
	if (choice === "daily") return `${time} * * *`;
	if (choice === "weekdays") return `${time} * * 1-5`;
	if (choice === "weekly") return `${time} * * ${start.getDay()}`;
	return `${time} ${start.getDate()} * *`;
}

export function repeatOf(
	cron: string | null | undefined,
	start: Date,
): RepeatChoice {
	const normalized = (cron ?? "").trim().split(/\s+/).join(" ");
	if (!normalized) return "none";
	for (const choice of ["daily", "weekdays", "weekly", "monthly"] as const)
		if (repeatCron(choice, start) === normalized) return choice;
	return "custom";
}

export interface EventDraft {
	title: string;
	allDay: boolean;
	/** `datetime-local` values, or `date` values for a whole-day event; an empty end has none. */
	start: string;
	end: string;
	location: string;
	notes: string;
	repeat: RepeatChoice;
	cron: string;
	wake: WakeChoice;
	offsets: number[];
}

export function newEventDraft(
	start: Date,
	end: Date,
	allDay: boolean,
): EventDraft {
	return {
		title: "",
		allDay,
		start: allDay ? toDateInput(start) : toDateTimeLocalInput(start),
		end: allDay ? toDateInput(end) : toDateTimeLocalInput(end),
		location: "",
		notes: "",
		repeat: "none",
		cron: "",
		wake: "default",
		offsets: [],
	};
}

/** The form for an event of the bot's own calendar. */
export function eventDraftOf(task: IBotTask): EventDraft | null {
	const event = task.event;
	const start = parseInstant(event?.starts_at);
	if (!event || start === null) return null;
	const end = parseInstant(event.ends_at);
	const span = event.all_day ? allDaySpan(start, end, true) : null;
	return {
		title: task.title,
		allDay: Boolean(event.all_day),
		start: span?.start ?? toDateTimeLocalInput(new Date(start)),
		end: span?.end ?? (end === null ? "" : toDateTimeLocalInput(new Date(end))),
		location: event.location ?? "",
		notes: task.instruction,
		repeat: repeatOf(task.cron, new Date(start)),
		cron: task.cron ?? "",
		wake: wakeChoiceOf(event.wake),
		offsets: event.wake?.offsets_minutes ?? [],
	};
}

export interface EventFields {
	title: string;
	instruction: string;
	/** Empty stops a repeating event from repeating. */
	cron: string;
	tz: string;
	event: IBotTaskEvent;
}

/** What a filled form saves, or nothing while it isn't ready. A whole-day event ends at the next midnight. */
export function eventFields(
	draft: EventDraft,
	tz: string,
	base?: IBotTaskEvent | null,
): EventFields | null {
	const title = draft.title.trim();
	if (!title || !draft.start) return null;
	let startsAt: Date;
	let endsAt: Date | null;
	if (draft.allDay) {
		startsAt = toDate(draft.start);
		const last = draft.end ? toDate(draft.end) : startsAt;
		if (last < startsAt) return null;
		endsAt = addDays(last, 1);
	} else {
		startsAt = new Date(draft.start);
		endsAt = draft.end ? new Date(draft.end) : null;
		if (endsAt && (Number.isNaN(endsAt.getTime()) || endsAt < startsAt))
			return null;
	}
	if (Number.isNaN(startsAt.getTime())) return null;
	const cron =
		draft.repeat === "none"
			? ""
			: draft.repeat === "custom"
				? draft.cron.trim()
				: repeatCron(draft.repeat, startsAt);
	const wake = eventWakeOf(draft.wake, draft.offsets);
	return {
		title,
		instruction: draft.notes.trim(),
		cron,
		tz,
		event: eventWithWake(
			{
				...(base ?? {}),
				starts_at: startsAt.toISOString(),
				ends_at: endsAt?.toISOString() ?? null,
				all_day: draft.allDay,
				location: draft.location.trim() || null,
			},
			wake,
		),
	};
}

/** Switches a form between whole days and times, keeping its days. */
export function withAllDay(draft: EventDraft, allDay: boolean): EventDraft {
	const startDay = draft.start.slice(0, 10);
	const endDay = draft.end.slice(0, 10);
	if (allDay) return { ...draft, allDay, start: startDay, end: endDay };
	return {
		...draft,
		allDay,
		start: startDay ? `${startDay}T09:00` : "",
		end: endDay ? `${endDay}T10:00` : "",
	};
}

/**
 * The whole event after it was dragged to `moved`'s times. A whole-day event ends at the next
 * midnight; a moment without an end keeps none unless it was resized.
 */
export function movedEvent(
	event: IBotTaskEvent,
	moved: CalendarEvent,
	resized: boolean,
): IBotTaskEvent {
	const start = toDate(moved.start);
	const end = moved.allDay
		? addDays(toDate(moved.end ?? moved.start), 1)
		: eventEnd(moved);
	const keepsEnd = Boolean(event.ends_at) || moved.allDay || resized;
	return eventWithWake(
		{
			...event,
			starts_at: start.toISOString(),
			ends_at: keepsEnd ? end.toISOString() : null,
		},
		event.wake ?? undefined,
	);
}

/** Adds a word unless it is blank or already there in any case; at most 20, each up to 80 characters. */
export function addWakePattern(
	patterns: readonly string[],
	text: string,
): string[] {
	const pattern = text.trim().slice(0, MAX_WAKE_PATTERN_CHARS);
	if (
		!pattern ||
		patterns.length >= MAX_WAKE_PATTERNS ||
		patterns.some((item) => item.toLowerCase() === pattern.toLowerCase())
	)
		return [...patterns];
	return [...patterns, pattern];
}

/** "Wed, Nov 4 · 09:00 – 10:00", "Wed, Nov 4 – Fri, Nov 6 · All day". */
export function formatWhen(
	event: CalendarEvent,
	locale: string | undefined,
	allDayLabel: string,
): string {
	const start = toDate(event.start);
	const end = event.allDay && event.end ? toDate(event.end) : eventEnd(event);
	const day = new Intl.DateTimeFormat(locale, {
		weekday: "short",
		month: "short",
		day: "numeric",
	});
	const time = new Intl.DateTimeFormat(locale, {
		hour: "2-digit",
		minute: "2-digit",
	});
	const sameDay = start.toDateString() === end.toDateString();
	if (event.allDay) {
		const days = sameDay
			? day.format(start)
			: `${day.format(start)} – ${day.format(end)}`;
		return `${days} · ${allDayLabel}`;
	}
	if (sameDay)
		return `${day.format(start)} · ${time.format(start)} – ${time.format(end)}`;
	return `${day.format(start)} ${time.format(start)} – ${day.format(end)} ${time.format(end)}`;
}

/** Sends the repeat and the zone only when they change, since moving either re-arms the wake-ups. */
export function updateEventRequest(
	fields: EventFields,
	task: Pick<IBotTask, "cron" | "tz">,
): IUpdateBotTaskRequest {
	const { cron, tz, ...rest } = fields;
	return {
		...rest,
		...(cron === (task.cron ?? "") ? {} : { cron }),
		...(tz === task.tz ? {} : { tz }),
	};
}

export function createEventRequest(fields: EventFields): ICreateBotTaskRequest {
	const { cron, ...rest } = fields;
	return { kind: "event", ...rest, ...(cron ? { cron } : {}) };
}
