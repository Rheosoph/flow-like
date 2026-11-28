import { describe, expect, test } from "bun:test";
import type { IBotTask } from "../../state/backend-state/bot-state";
import {
	describeSchedule,
	emptyTaskDraft,
	scheduledTasks,
	taskRequest,
	taskState,
} from "./bot-task-model";

function task(patch: Partial<IBotTask>): IBotTask {
	return {
		id: "t1",
		bot_id: "b1",
		kind: "once",
		instruction: "Check the report",
		title: "Report",
		enabled: true,
		created_by: "owner",
		created_at: "2026-11-03T08:00:00Z",
		...patch,
	};
}

describe("taskRequest", () => {
	test("monitoring is explicit and reminders keep their normal delivery", () => {
		const draft = {
			...emptyTaskDraft(),
			at: "2026-11-04T09:30",
			instruction: "Check the report",
		};
		expect(taskRequest(draft)?.monitoring).toBeUndefined();
		expect(taskRequest({ ...draft, monitoring: true })?.monitoring).toBe(true);
		expect(
			taskRequest({
				...draft,
				kind: "cron",
				cron: "0 9 * * *",
				monitoring: true,
			})?.monitoring,
		).toBe(true);
		expect(
			taskRequest({ ...draft, monitoring: false })?.monitoring,
		).toBeUndefined();
	});

	test("a once task needs an instruction and a time, sent as an instant", () => {
		const draft = {
			...emptyTaskDraft(),
			instruction: " Check the report ",
			at: "2026-11-04T09:30",
		};
		const request = taskRequest(draft);
		expect(request?.kind).toBe("once");
		expect(request?.instruction).toBe("Check the report");
		expect(request?.at).toBe(new Date("2026-11-04T09:30").toISOString());
		expect(taskRequest({ ...draft, at: "" })).toBeUndefined();
		expect(taskRequest({ ...draft, instruction: "  " })).toBeUndefined();
	});

	test("a cron task needs five fields and keeps its time zone", () => {
		const draft = {
			...emptyTaskDraft("Europe/Berlin"),
			kind: "cron" as const,
			cron: "  0  9 * *   1-5 ",
			instruction: "Morning brief",
		};
		expect(taskRequest(draft)).toEqual({
			kind: "cron",
			cron: "0 9 * * 1-5",
			tz: "Europe/Berlin",
			instruction: "Morning brief",
			title: undefined,
		});
		expect(taskRequest({ ...draft, cron: "0 9 * *" })).toBeUndefined();
	});
});

describe("task display", () => {
	test("a cron task reads as its expression and zone", () => {
		expect(
			describeSchedule(
				task({ kind: "cron", cron: "0 9 * * *", tz: "Europe/Berlin" }),
			),
		).toBe("0 9 * * * (Europe/Berlin)");
	});

	test("paused and finished tasks are told apart", () => {
		expect(taskState(task({}))).toBe("active");
		expect(taskState(task({ enabled: false }))).toBe("paused");
		expect(
			taskState(task({ enabled: false, last_run_at: "2026-11-04T09:00:00Z" })),
		).toBe("done");
		expect(
			taskState(
				task({
					kind: "cron",
					enabled: false,
					last_run_at: "2026-11-04T09:00Z",
				}),
			),
		).toBe("paused");
	});
});

describe("place tasks", () => {
	const place = (patch: Partial<NonNullable<IBotTask["place"]>> = {}) =>
		task({
			kind: "place",
			title: "Groceries",
			place: {
				latitude: 52.5,
				longitude: 13.4,
				radius_m: 150,
				on: "enter",
				label: "Home",
				repeat: false,
				...patch,
			},
		});

	test("reads like the trigger, with its repeat and end", () => {
		expect(describeSchedule(place(), "en")).toBe(
			"When you arrive at Home · once",
		);
		expect(
			describeSchedule(place({ on: "exit", label: "Office", repeat: true })),
		).toBe("When you leave Office · every time");
		expect(describeSchedule(place({ on: "both", label: "Gym" }))).toBe(
			"When you arrive at or leave Gym · once",
		);
		const until = "2026-11-20T18:00:00Z";
		const text = describeSchedule(place({ until }), "en");
		expect(text).toContain("until ");
		expect(text).toContain("2026");
	});

	test("watching, done, expired and paused", () => {
		const now = Date.parse("2026-11-19T12:00:00Z");
		expect(taskState(place(), now)).toBe("active");
		expect(
			taskState(
				{ ...place(), enabled: false, last_run_at: "2026-11-19T10:00:00Z" },
				now,
			),
		).toBe("done");
		expect(
			taskState(
				{ ...place({ repeat: true }), last_run_at: "2026-11-19T10:00:00Z" },
				now,
			),
		).toBe("active");
		expect(taskState(place({ until: "2026-11-19T11:00:00Z" }), now)).toBe(
			"expired",
		);
		expect(taskState(place({ until: "2026-11-19T13:00:00Z" }), now)).toBe(
			"active",
		);
		expect(taskState({ ...place(), enabled: false }, now)).toBe("paused");
		expect(
			taskState(
				{
					...place({ repeat: true }),
					enabled: false,
					last_run_at: "2026-11-19T10:00:00Z",
				},
				now,
			),
		).toBe("paused");
	});
});

describe("scheduledTasks", () => {
	test("leaves the bot's calendar events to the Calendar tab", () => {
		const tasks = [
			task({ id: "once" }),
			task({ id: "cron", kind: "cron" }),
			task({ id: "place", kind: "place" }),
			task({ id: "event", kind: "event" }),
		];
		expect(scheduledTasks(tasks).map((item) => item.id)).toEqual([
			"once",
			"cron",
			"place",
		]);
	});
});
