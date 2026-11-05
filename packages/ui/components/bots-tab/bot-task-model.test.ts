import { describe, expect, test } from "bun:test";
import type { IBotTask } from "../../state/backend-state/bot-state";
import {
	describeSchedule,
	emptyTaskDraft,
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
