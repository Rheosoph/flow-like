"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ScheduleRun,
	scheduleRun,
} from "../../../../lib/device-management/model/schedule-where";
import type {
	ServiceBot,
	ServiceBotState,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { eventTypeLabel } from "../copy/eligibility-copy";
import { scheduleTime, scheduleZone } from "../copy/schedule-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { botRows } from "./bots-block";
import { type ScheduleRow, scheduleRows } from "./schedules-block";
import { type ListRun, listRun } from "./service-events";
import type { ServiceApp } from "./service-header";

/* One-line facts about a service's one-time schedules and bots, for a row in a list of services. */

const LINE = "basis-full text-xs text-muted-foreground";

/** "Runs once …", "Ran once …" or "… didn't run", else the time its event saves. */
function onceText(
	t: DevicesT,
	run: ScheduleRun,
	row: ScheduleRow,
	time: AreaTime,
) {
	if (run.state === "finished") {
		const at = scheduleTime(t, run.finished.at, run.entry.timezone, time);
		return run.finished.state === "ran"
			? t("devices:device.services.onceRan", "Ran once {{time}}", { time: at })
			: t(
					"devices:device.services.onceNotRun",
					"One-time schedule didn't run ({{time}})",
					{ time: at },
				);
	}
	if (run.state === "armed" && run.next)
		return t(
			"devices:serviceStatus.schedules.once.willRun",
			"Runs once {{time}}",
			{
				time: scheduleTime(t, run.next.at, run.entry.timezone, time),
			},
		);
	return row.once
		? t(
				"devices:eligibility.runs.onceAt",
				"Once on {{date}} at {{time}} · {{zone}}",
				{
					date: row.once.date,
					time: row.once.time,
					zone: scheduleZone(t, row.once),
				},
			)
		: null;
}

interface LinesProps {
	service: ServiceView;
	app: Pick<ServiceApp, "view">;
}

/** One line per one-time schedule of the service: when it runs, or how it ended. */
export function OnceLines({ service, app }: Readonly<LinesProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const lineOf = (row: ScheduleRow) => {
		const run = scheduleRun(service, row.eventId, time.nowS);
		const text = row.once ? onceText(t, run, row, time) : null;
		return text ? [{ id: row.eventId, text }] : [];
	};
	const lines = scheduleRows(service, app).flatMap(lineOf);
	return (
		<>
			{lines.map((line) => (
				<span key={line.id} data-once-line={line.id} className={LINE}>
					{line.text}
				</span>
			))}
		</>
	);
}

type BotWord =
	| "connected"
	| "connecting"
	| "off"
	| "token"
	| "intents"
	| "elsewhere";

const BOT_WORD: Partial<Record<ServiceBotState, BotWord>> = {
	connected: "connected",
	ok: "connected",
	connecting: "connecting",
	reconnecting: "connecting",
	waiting: "off",
	token_refused: "token",
	intents_refused: "intents",
	conflict: "elsewhere",
	webhook_set: "elsewhere",
};

const BOT_WORDS: Record<BotWord, (t: DevicesT) => string> = {
	connected: (t) =>
		t("devices:device.services.botState.connected", "connected"),
	connecting: (t) =>
		t("devices:device.services.botState.connecting", "connecting"),
	off: (t) => t("devices:device.services.botState.off", "not connected"),
	token: (t) =>
		t("devices:device.services.botState.tokenRefused", "token refused"),
	intents: (t) =>
		t("devices:device.services.botState.intentsRefused", "permissions refused"),
	elsewhere: (t) =>
		t("devices:device.services.botState.elsewhere", "used elsewhere"),
};

/** A bot's state in a word or two; null while nothing is known about it. */
function botWord(run: ListRun<ServiceBot>) {
	if (run.state === "stopped") return "off" as const;
	if (run.state !== "reported") return null;
	return run.entry.hold
		? ("off" as const)
		: (BOT_WORD[run.entry.state] ?? null);
}

/** "Telegram bot · connected; Discord bot · token refused" for the bots of the service. */
export function BotLine({ service, app }: Readonly<LinesProps>) {
	const { t } = useTranslation("devices");
	const partOf = (row: ReturnType<typeof botRows>[number]) => {
		const type = eventTypeLabel(t, row.provider);
		const word = botWord(listRun(service, service.bots, row.eventId));
		return word
			? t("devices:device.services.bot", "{{type}} · {{state}}", {
					type,
					state: BOT_WORDS[word](t),
				})
			: type;
	};
	const parts = botRows(service, app).map(partOf);
	if (!parts.length) return null;
	return (
		<span data-bots-line="" className={LINE}>
			{parts.join("; ")}
		</span>
	);
}
