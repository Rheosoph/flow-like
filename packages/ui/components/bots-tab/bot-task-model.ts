import type {
	IBotTask,
	ICreateBotTaskRequest,
} from "../../state/backend-state/bot-state";

/** Tasks a bot keeps, its calendar events apart; the server enforces it. */
export const MAX_BOT_TASKS = 50;

/** Event tasks are the bot's calendar: the Calendar tab shows them, task lists leave them out. */
export function scheduledTasks(tasks: readonly IBotTask[]): IBotTask[] {
	return tasks.filter((task) => task.kind !== "event");
}

export type TaskDraft = {
	monitoring?: boolean;
	kind: "once" | "cron";
	/** `datetime-local` value: `YYYY-MM-DDTHH:mm` in the viewer's zone. */
	at: string;
	cron: string;
	tz: string;
	instruction: string;
	title: string;
};

export function emptyTaskDraft(tz = "UTC"): TaskDraft {
	return {
		kind: "once",
		at: "",
		cron: "",
		tz,
		instruction: "",
		title: "",
		monitoring: false,
	};
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
		return {
			kind: "once",
			at: at.toISOString(),
			instruction,
			title,
			...(draft.monitoring ? { monitoring: true } : {}),
		};
	}
	const cron = draft.cron.trim().split(/\s+/).join(" ");
	if (cron.split(" ").length !== 5) return;
	return {
		kind: "cron",
		...(draft.monitoring ? { monitoring: true } : {}),
		cron,
		tz: draft.tz.trim() || "UTC",
		instruction,
		title,
	};
}

export type TaskTranslate = (
	key: string,
	fallback: string,
	options?: Record<string, unknown>,
) => string;

const englishText: TaskTranslate = (_key, fallback, options) =>
	fallback.replace(/{{(\w+)}}/g, (_, name) => String(options?.[name] ?? ""));

function formatMoment(value: string, locale?: string): string | undefined {
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return;
	return new Intl.DateTimeFormat(locale, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(date);
}

/** "When you arrive at Home · every time · until Nov 4, 9:00". */
export function describePlace(
	task: IBotTask,
	locale?: string,
	t: TaskTranslate = englishText,
): string {
	const place = task.place;
	if (!place) return "";
	const options = { place: place.label };
	const trigger =
		place.on === "enter"
			? t("panel.tasks.place.enter", "When you arrive at {{place}}", options)
			: place.on === "exit"
				? t("panel.tasks.place.exit", "When you leave {{place}}", options)
				: t(
						"panel.tasks.place.both",
						"When you arrive at or leave {{place}}",
						options,
					);
	const parts = [
		trigger,
		place.repeat
			? t("panel.tasks.place.repeat", "every time")
			: t("panel.tasks.place.once", "once"),
	];
	const until = place.until ? formatMoment(place.until, locale) : undefined;
	if (until)
		parts.push(t("panel.tasks.place.until", "until {{when}}", { when: until }));
	return parts.join(" · ");
}

/** "Once, Nov 4, 9:00", "0 9 * * 1-5 (Europe/Berlin)" or a place description. */
export function describeSchedule(
	task: IBotTask,
	locale?: string,
	t?: TaskTranslate,
): string {
	if (task.kind === "place") return describePlace(task, locale, t);
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

export type TaskState = "active" | "paused" | "done" | "expired";

/** Paused tasks and finished once tasks have no next run; a place task past its end is expired. */
export function taskState(task: IBotTask, now: number = Date.now()): TaskState {
	const place = task.kind === "place" ? task.place : undefined;
	if (place) {
		if (!place.repeat && task.last_run_at) return "done";
		const until = place.until ? new Date(place.until).getTime() : Number.NaN;
		if (!Number.isNaN(until) && until <= now) return "expired";
	}
	if (task.enabled) return "active";
	return task.kind === "once" && task.last_run_at ? "done" : "paused";
}
