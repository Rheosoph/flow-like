import { describe, expect, test } from "bun:test";
import type {
	IBotTask,
	ICalendarEntry,
} from "../../state/backend-state/bot-state";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";
import {
	CALENDAR_LAYERS,
	type CalendarLayer,
	addWakePattern,
	calendarItems,
	calendarWindow,
	createEventRequest,
	eventDraftOf,
	eventFields,
	eventWakeOf,
	eventWithWake,
	externalRefOf,
	externalWakeRequest,
	formatWakeOffset,
	formatWakeOffsets,
	memoryItems,
	movedEvent,
	newEventDraft,
	repeatCron,
	repeatOf,
	taskOccurrences,
	toggleWakeOffset,
	updateEventRequest,
	wakeChoiceOf,
	withAllDay,
} from "./bot-calendar-model";

const ALL = new Set<CalendarLayer>(CALENDAR_LAYERS);
const NOW = Date.parse("2026-11-20T12:00:00Z");
const WEEK = {
	from: Date.parse("2026-11-16T00:00:00Z"),
	to: Date.parse("2026-11-23T00:00:00Z"),
};

function entry(patch: Partial<ICalendarEntry>): ICalendarEntry {
	return {
		id: "task_1",
		source: "bot",
		title: "Dentist",
		starts_at: "2026-11-18T09:00:00Z",
		ends_at: "2026-11-18T10:00:00Z",
		task_id: "task_1",
		...patch,
	};
}

function task(patch: Partial<IBotTask>): IBotTask {
	return {
		id: "t1",
		bot_id: "b1",
		kind: "once",
		instruction: "Remind me",
		title: "Report",
		enabled: true,
		created_by: "owner",
		created_at: "2026-11-01T00:00:00Z",
		...patch,
	};
}

function fact(patch: Partial<BotFact>): BotFact {
	return {
		id: "f1",
		text: "Anna's birthday",
		kind: "event",
		subject: "user",
		source: "user_stated",
		confidence: 1,
		recorded_at: "2026-11-01T00:00:00Z",
		valid_from: null,
		valid_to: null,
		superseded_by: null,
		evidence: [],
		...patch,
	};
}

describe("layering the calendar", () => {
	test("own and connected entries keep their layer, color and a unique id per occurrence", () => {
		const items = calendarItems(
			{
				range: {
					from: "",
					to: "",
					sources: [],
					entries: [
						entry({}),
						entry({ starts_at: "2026-11-19T09:00:00Z", recurring: true }),
						entry({
							id: "ext.google.c1.70.65",
							source: "external",
							title: "Standup",
							task_id: null,
						}),
					],
				},
			},
			ALL,
			WEEK,
			NOW,
			true,
		);
		expect(items.map((item) => item.subject.layer)).toEqual([
			"events",
			"events",
			"external",
		]);
		expect(new Set(items.map((item) => item.event.id)).size).toBe(3);
		expect(items[0].event).toMatchObject({
			title: "Dentist",
			allDay: false,
			color: "var(--primary)",
			editable: true,
		});
		expect(items[1].event.editable).toBe(false);
		expect(items[2].event).toMatchObject({
			color: "var(--info-solid)",
			editable: false,
		});
	});

	test("an own event isn't editable without the owner's right to change it", () => {
		const [item] = calendarItems(
			{ range: { from: "", to: "", sources: [], entries: [entry({})] } },
			ALL,
			WEEK,
			NOW,
			false,
		);
		expect(item.event.editable).toBe(false);
	});

	test("a whole-day entry ends on its last day, not the exclusive midnight after it", () => {
		const local = new Date(2026, 10, 18).getTime();
		const [item] = calendarItems(
			{
				range: {
					from: "",
					to: "",
					sources: [],
					entries: [
						entry({
							all_day: true,
							starts_at: new Date(local).toISOString(),
							ends_at: new Date(local + 2 * 86_400_000).toISOString(),
						}),
					],
				},
			},
			ALL,
			WEEK,
			NOW,
			true,
		);
		expect(item.event).toMatchObject({
			allDay: true,
			start: "2026-11-18",
			end: "2026-11-19",
		});
	});

	test("hidden layers and items outside the window stay out", () => {
		const inputs = {
			range: {
				from: "",
				to: "",
				sources: [],
				entries: [entry({}), entry({ starts_at: "2026-12-24T09:00:00Z" })],
			},
			tasks: [task({ at: "2026-11-21T08:00:00Z" })],
		};
		expect(
			calendarItems(inputs, new Set(["tasks"]), WEEK, NOW, true).map(
				(item) => item.subject.layer,
			),
		).toEqual(["tasks"]);
		expect(calendarItems(inputs, ALL, WEEK, NOW, true)).toHaveLength(2);
	});

	test("task lists skip event tasks; place tasks have no time", () => {
		const items = calendarItems(
			{
				tasks: [
					task({ id: "e", kind: "event", at: "2026-11-21T08:00:00Z" }),
					task({ id: "p", kind: "place" }),
					task({ id: "o", at: "2026-11-21T08:00:00Z" }),
				],
			},
			ALL,
			WEEK,
			NOW,
			true,
		);
		expect(items.map((item) => item.event.id)).toEqual([
			"tasks:o:2026-11-21T08:00:00.000Z",
		]);
		expect(items[0].event.color).toBe("var(--warning-solid)");
	});
});

describe("task occurrences", () => {
	test("a once task shows at its time inside the window", () => {
		expect(
			taskOccurrences(task({ at: "2026-11-21T08:00:00Z" }), WEEK.from, WEEK.to),
		).toEqual(["2026-11-21T08:00:00.000Z"]);
		expect(
			taskOccurrences(task({ at: "2026-12-21T08:00:00Z" }), WEEK.from, WEEK.to),
		).toEqual([]);
	});

	test("an active cron task repeats from its next run to the window's end", () => {
		const daily = task({
			kind: "cron",
			cron: "0 9 * * *",
			tz: "UTC",
			next_run_at: "2026-11-20T09:00:00Z",
		});
		expect(taskOccurrences(daily, WEEK.from, WEEK.to)).toEqual([
			"2026-11-20T09:00:00.000Z",
			"2026-11-21T09:00:00.000Z",
			"2026-11-22T09:00:00.000Z",
		]);
		expect(
			taskOccurrences({ ...daily, enabled: false }, WEEK.from, WEEK.to),
		).toEqual([]);
	});

	test("a cron the parser refuses falls back to the next run", () => {
		expect(
			taskOccurrences(
				task({
					kind: "cron",
					cron: "not a cron",
					next_run_at: "2026-11-20T09:00:00Z",
				}),
				WEEK.from,
				WEEK.to,
			),
		).toEqual(["2026-11-20T09:00:00.000Z"]);
	});
});

describe("remembered dates", () => {
	test("a date said without a time is a whole day; a time is a short moment", () => {
		const day = new Date(2026, 10, 21).toISOString();
		expect(memoryItems(fact({ time: day }), NOW)[0].event).toMatchObject({
			allDay: true,
			start: "2026-11-21",
			color: "var(--locked-solid)",
		});
		const [moment] = memoryItems(fact({ time: "2026-11-21T14:30:00Z" }), NOW);
		expect(moment.event).toMatchObject({
			allDay: false,
			start: "2026-11-21T14:30:00.000Z",
			end: "2026-11-21T15:00:00.000Z",
		});
	});

	test("a validity span is one item; a long one shows where it starts and ends", () => {
		const from = new Date(2026, 11, 1).toISOString();
		const to = new Date(2026, 11, 5).toISOString();
		const [span] = memoryItems(fact({ valid_from: from, valid_to: to }), NOW);
		expect(span.event).toMatchObject({
			allDay: true,
			start: "2026-12-01",
			end: "2026-12-05",
		});
		const long = memoryItems(
			fact({
				valid_from: new Date(2026, 0, 1).toISOString(),
				valid_to: new Date(2026, 11, 31).toISOString(),
			}),
			NOW,
		);
		expect(long.map((item) => item.event.id)).toEqual([
			"memories:f1:from",
			"memories:f1:to",
		]);
	});

	test("superseded, expired and undated facts stay out", () => {
		const time = "2026-11-21T14:30:00Z";
		expect(memoryItems(fact({ time, superseded_by: "f2" }), NOW)).toEqual([]);
		expect(
			memoryItems(fact({ time, expires_at: "2026-11-19T00:00:00Z" }), NOW),
		).toEqual([]);
		expect(memoryItems(fact({}), NOW)).toEqual([]);
	});
});

describe("listing windows", () => {
	test("cover the days on screen, at most 92 days", () => {
		expect(
			calendarWindow({
				from: new Date("2026-11-16T00:00:00Z"),
				to: new Date("2026-11-23T00:00:00Z"),
			}),
		).toEqual({
			from: "2026-11-16T00:00:00.000Z",
			to: "2026-11-23T00:00:00.000Z",
		});
		expect(
			calendarWindow({
				from: new Date("2026-01-01T00:00:00Z"),
				to: new Date("2026-12-31T00:00:00Z"),
			}).to,
		).toBe("2026-04-03T00:00:00.000Z");
	});
});

describe("wake-ups", () => {
	test("offsets read in the viewer's language", () => {
		expect(formatWakeOffset(15, "en")).toBe("15 min");
		expect(formatWakeOffset(120, "en")).toBe("2 hr");
		expect(formatWakeOffset(1440, "en")).toBe("1 day");
		expect(formatWakeOffsets({ offsets_minutes: [1440, 15] }, "en")).toBe(
			"15 min, 1 day",
		);
	});

	test("an event follows the settings, wakes at its own times or never", () => {
		expect(wakeChoiceOf(undefined)).toBe("default");
		expect(wakeChoiceOf({ enabled: false })).toBe("off");
		expect(wakeChoiceOf({ enabled: true, offsets_minutes: [] })).toBe("on");
		expect(eventWakeOf("default", [15])).toBeUndefined();
		expect(eventWakeOf("off", [15])).toEqual({
			enabled: false,
			offsets_minutes: [],
		});
		expect(eventWakeOf("on", [15, 60, 15])).toEqual({
			enabled: true,
			offsets_minutes: [60, 15],
		});
	});

	test("at most four offsets, largest first", () => {
		let offsets: number[] = [];
		for (const minutes of [5, 60, 15, 1440, 30])
			offsets = toggleWakeOffset(offsets, minutes);
		expect(offsets).toEqual([1440, 60, 15, 5]);
		expect(toggleWakeOffset(offsets, 60)).toEqual([1440, 15, 5]);
	});

	test("patterns are trimmed, unique in any case and limited", () => {
		expect(addWakePattern(["Standup"], "  standup ")).toEqual(["Standup"]);
		expect(addWakePattern(["Standup"], " 1:1 ")).toEqual(["Standup", "1:1"]);
		expect(addWakePattern([], "   ")).toEqual([]);
		expect(addWakePattern([], "x".repeat(100))[0]).toHaveLength(80);
	});

	test("a new wake-up replaces the old one and drops the resolved offsets", () => {
		const event = {
			starts_at: "2026-11-20T09:00:00Z",
			wake: { enabled: false },
			wake_offsets: [15],
		};
		expect(eventWithWake(event, undefined)).toEqual({
			starts_at: "2026-11-20T09:00:00Z",
		});
		expect(
			eventWithWake(event, { enabled: true, offsets_minutes: [30] }),
		).toEqual({
			starts_at: "2026-11-20T09:00:00Z",
			wake: { enabled: true, offsets_minutes: [30] },
		});
	});
});

describe("connected-calendar events", () => {
	test("the entry id decodes to the event reference", () => {
		expect(
			externalRefOf("ext.google.conn_1.7072696d617279.6576656e742d31"),
		).toEqual({
			connector_id: "conn_1",
			provider: "google",
			calendar_id: "primary",
			event_id: "event-1",
		});
		expect(externalRefOf("task_1")).toBeNull();
		expect(externalRefOf("ext.google.conn.zz.00")).toBeNull();
		expect(externalRefOf("ext.google.conn..61")).toBeNull();
	});

	test("a wake-up for one creates an event task keyed by its reference", () => {
		const request = externalWakeRequest(
			entry({
				id: "ext.microsoft.c2.6361.6576",
				source: "external",
				link: "https://outlook.office.com/e",
			}),
			{ enabled: true, offsets_minutes: [10] },
			"Europe/Berlin",
		);
		expect(request).toEqual({
			kind: "event",
			title: "Dentist",
			instruction: "",
			tz: "Europe/Berlin",
			event: {
				starts_at: "2026-11-18T09:00:00Z",
				ends_at: "2026-11-18T10:00:00Z",
				all_day: false,
				location: null,
				link: "https://outlook.office.com/e",
				external: {
					connector_id: "c2",
					provider: "microsoft",
					calendar_id: "ca",
					event_id: "ev",
				},
				wake: { enabled: true, offsets_minutes: [10] },
			},
		});
		expect(externalWakeRequest(entry({}), undefined, "UTC")).toBeNull();
	});
});

describe("the event form", () => {
	const start = new Date(2026, 10, 20, 9, 30);

	test("repeat presets follow the start's time and day", () => {
		expect(repeatCron("daily", start)).toBe("30 9 * * *");
		expect(repeatCron("weekdays", start)).toBe("30 9 * * 1-5");
		expect(repeatCron("weekly", start)).toBe("30 9 * * 5");
		expect(repeatCron("monthly", start)).toBe("30 9 20 * *");
		expect(repeatOf("30  9 * * 1-5", start)).toBe("weekdays");
		expect(repeatOf("*/5 * * * *", start)).toBe("custom");
		expect(repeatOf(null, start)).toBe("none");
	});

	test("a timed event saves its times, a repeat and its own wake-up", () => {
		const draft = {
			...newEventDraft(start, new Date(2026, 10, 20, 10, 30), false),
			title: " Review ",
			notes: " Bring the numbers ",
			location: " Room 4 ",
			repeat: "weekly" as const,
			wake: "on" as const,
			offsets: [15],
		};
		const fields = eventFields(draft, "Europe/Berlin");
		expect(fields).toEqual({
			title: "Review",
			instruction: "Bring the numbers",
			cron: "30 9 * * 5",
			tz: "Europe/Berlin",
			event: {
				starts_at: start.toISOString(),
				ends_at: new Date(2026, 10, 20, 10, 30).toISOString(),
				all_day: false,
				location: "Room 4",
				wake: { enabled: true, offsets_minutes: [15] },
			},
		});
		expect(createEventRequest(fields as never)).toMatchObject({
			kind: "event",
			cron: "30 9 * * 5",
		});
	});

	test("an edit sends the repeat and the zone only when they change", () => {
		const fields = {
			title: "Review",
			instruction: "",
			cron: "",
			tz: "Europe/Berlin",
			event: { starts_at: "2026-11-20T09:00:00Z" },
		};
		expect(updateEventRequest(fields, { tz: "Europe/Berlin" })).toEqual({
			title: "Review",
			instruction: "",
			event: { starts_at: "2026-11-20T09:00:00Z" },
		});
		expect(
			updateEventRequest(fields, { cron: "0 9 * * 5", tz: "UTC" }),
		).toMatchObject({ cron: "", tz: "Europe/Berlin" });
	});

	test("a whole-day event ends at the midnight after its last day", () => {
		const fields = eventFields(
			{
				...newEventDraft(start, start, true),
				title: "Trip",
				end: "2026-11-22",
			},
			"UTC",
		);
		expect(fields?.event).toMatchObject({
			all_day: true,
			starts_at: new Date(2026, 10, 20).toISOString(),
			ends_at: new Date(2026, 10, 23).toISOString(),
		});
		expect(fields?.cron).toBe("");
		expect(createEventRequest(fields as never)).not.toHaveProperty("cron");
	});

	test("isn't ready without a title or with an end before the start", () => {
		const draft = newEventDraft(start, start, false);
		expect(eventFields(draft, "UTC")).toBeNull();
		expect(
			eventFields({ ...draft, title: "X", end: "2026-11-19T09:00" }, "UTC"),
		).toBeNull();
	});

	test("switching to whole days keeps the days", () => {
		const draft = newEventDraft(start, new Date(2026, 10, 21, 11, 0), false);
		const days = withAllDay(draft, true);
		expect([days.start, days.end]).toEqual(["2026-11-20", "2026-11-21"]);
		expect(withAllDay(days, false)).toMatchObject({
			start: "2026-11-20T09:00",
			end: "2026-11-21T10:00",
		});
	});

	test("an own event opens in the form as it was saved", () => {
		const draft = eventDraftOf(
			task({
				kind: "event",
				title: "Trip",
				instruction: "Pack",
				cron: "0 0 * * 5",
				tz: "Europe/Berlin",
				event: {
					starts_at: new Date(2026, 10, 20).toISOString(),
					ends_at: new Date(2026, 10, 22).toISOString(),
					all_day: true,
					wake: { enabled: false },
				},
			}),
		);
		expect(draft).toMatchObject({
			title: "Trip",
			notes: "Pack",
			allDay: true,
			start: "2026-11-20",
			end: "2026-11-21",
			repeat: "weekly",
			wake: "off",
		});
	});

	test("a dragged event keeps its wake-up and gains an end only when resized", () => {
		const event = {
			starts_at: "2026-11-20T09:00:00Z",
			wake: { enabled: true, offsets_minutes: [5] },
			wake_offsets: [5],
		};
		const moved = {
			id: "x",
			title: "x",
			start: "2026-11-21T09:00:00.000Z",
			end: "2026-11-21T09:30:00.000Z",
		};
		expect(movedEvent(event, moved, false)).toEqual({
			starts_at: "2026-11-21T09:00:00.000Z",
			ends_at: null,
			wake: { enabled: true, offsets_minutes: [5] },
		});
		expect(movedEvent(event, moved, true).ends_at).toBe(
			"2026-11-21T09:30:00.000Z",
		);
	});
});
