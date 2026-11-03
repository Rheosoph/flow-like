import type {
	IBotTask,
	ICreateBotTaskRequest,
} from "../../state/backend-state/bot-state";

/** Tasks a bot keeps; the server enforces it. */
export const MAX_BOT_TASKS = 50;

export type TaskDraft = {
	kind: "once" | "cron";
	/** `datetime-local` value: `YYYY-MM-DDTHH:mm` in the viewer's zone. */
	at: string;
	cron: string;
	tz: string;
	instruction: string;
	title: string;
};

export function emptyTaskDraft(tz = "UTC"): TaskDraft {
	return { kind: "once", at: "", cron: "", tz, instruction: "", title: "" };
}

export function browserTimeZone(): string {
	try {
		return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
	} catch {
		return "UTC";
	}
}

/** The request a filled form stands for, or why it is not ready. */
export function taskRequest(
	draft: TaskDraft,
): ICreateBotTaskRequest | undefined {
	const instruction = draft.instruction.trim();
	if (!instruction) return;
	const title = draft.title.trim() || undefined;
	if (draft.kind === "once") {
		const at = new Date(draft.at);
		if (!draft.at || Number.isNaN(at.getTime())) return;
		return { kind: "once", at: at.toISOString(), instruction, title };
	}
	const cron = draft.cron.trim().split(/\s+/).join(" ");
	if (cron.split(" ").length !== 5) return;
	return {
		kind: "cron",
		cron,
		tz: draft.tz.trim() || "UTC",
		instruction,
		title,
	};
}

/** "Once, Nov 4, 9:00" or "0 9 * * 1-5 (Europe/Berlin)". */
export function describeSchedule(task: IBotTask, locale?: string): string {
	if (task.kind === "cron") {
		return `${task.cron ?? ""} (${task.tz ?? "UTC"})`;
	}
	const at = task.at ? new Date(task.at) : undefined;
	if (!at || Number.isNaN(at.getTime())) return "";
	return new Intl.DateTimeFormat(locale, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(at);
}

/** Paused tasks and finished once tasks have no next run. */
export function taskState(task: IBotTask): "active" | "paused" | "done" {
	if (task.enabled) return "active";
	return task.kind === "once" && task.last_run_at ? "done" : "paused";
}
