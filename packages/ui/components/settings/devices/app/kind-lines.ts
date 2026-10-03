import type { EventKind } from "../../../../lib/device-management/deployment";
import type {
	AppView,
	MatrixRow,
} from "../../../../lib/device-management/model/app-plan";
import {
	type BotRun,
	botRun,
	scheduleRun,
} from "../../../../lib/device-management/model/schedule-where";
import type {
	ServiceBot,
	ServiceBotState,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	type ScheduleRunNames,
	scheduleHoldReason,
	scheduleRunLines,
} from "../copy/schedule-copy";
import type { AreaTime, DevicesT } from "../primitives/area-context";

/* The kinds of round two in a service's cell: a bot's state, which events of a service a person starts. */

const BOT_STATES: Record<
	ServiceBotState,
	(t: DevicesT, bot: ServiceBot) => string
> = {
	connected: (t, bot) =>
		bot.bot_name
			? t("devices:app.bot.connectedAs", "Connected as {{name}}", {
					name: bot.bot_name,
				})
			: t("devices:app.bot.connected", "Connected"),
	ok: (t) => t("devices:app.bot.connected", "Connected"),
	connecting: (t) => t("devices:app.bot.connecting", "Connecting"),
	reconnecting: (t) => t("devices:app.bot.reconnecting", "Reconnecting"),
	waiting: (t) => t("devices:app.bot.waiting", "Waiting to connect"),
	token_refused: (t) =>
		t("devices:app.bot.tokenRefused", "Not connected: its token was refused"),
	intents_refused: (t) =>
		t(
			"devices:app.bot.intentsRefused",
			"Not connected: Discord refused its permissions",
		),
	conflict: (t) =>
		t(
			"devices:app.bot.conflict",
			"Not connected: another program uses its token",
		),
	webhook_set: (t) =>
		t(
			"devices:app.bot.webhookSet",
			"Not connected: Telegram sends its messages to a webhook",
		),
};

/** One bot of a service in one line: connected, trying to, or why not. A bot's name is the device's plain text. */
export function botLine(t: DevicesT, run: BotRun): string {
	switch (run.state) {
		case "stopped":
			return t(
				"devices:app.bot.stopped",
				"Not connected while the service is not running",
			);
		case "not_reported":
			return t("devices:app.bot.notReported", "Not reported yet");
		case "needs_agent":
			return t(
				"devices:app.bot.needsAgent",
				"Update the device agent to see its bots",
			);
		case "unknown":
			return t(
				"devices:app.bot.unknown",
				"Not in this status. Connect live to see whether it is connected.",
			);
		default:
			return run.bot.hold
				? t("devices:app.bot.held", "Not connected: {{reason}}", {
						reason: scheduleHoldReason(t, run.bot.hold, true),
					})
				: BOT_STATES[run.bot.state](t, run.bot);
	}
}

/**
 * The one line a served cell adds for its event's kind: when a schedule of
 * either kind runs next (or how a one-time schedule ended), a bot's state.
 */
export function cellLines(
	t: DevicesT,
	row: Pick<MatrixRow, "eventId" | "eligibility">,
	view: ServiceView,
	names: () => ScheduleRunNames,
	time: AreaTime,
): string[] {
	const { kind, schedule, once } = row.eligibility;
	if (kind === "bot") return [botLine(t, botRun(view, row.eventId))];
	if (!schedule && !once) return [];
	return scheduleRunLines(
		t,
		scheduleRun(view, row.eventId, time.nowS),
		names(),
		time,
	).slice(0, 1);
}

/** Whether a device answers requests for this kind on the service's address. */
export const answersRequests = (kind: EventKind | null): boolean =>
	kind === "served" || kind === "own_server";

/** The app's rule rows by event id: runnable ones and those that can't run any more. */
export function ruleRows(view: AppView): ReadonlyMap<string, MatrixRow> {
	return new Map(
		[...view.events.rows, ...view.events.ineligible].map((row) => [
			row.eventId,
			row,
		]),
	);
}

/** The events of a service that run as this kind, in the order the service lists them. */
export function eventsOfKind(
	rows: ReadonlyMap<string, MatrixRow>,
	served: readonly { event_id: string }[],
	kind: EventKind,
): MatrixRow[] {
	return served.flatMap((event) => {
		const row = rows.get(event.event_id);
		return row?.eligibility.kind === kind ? [row] : [];
	});
}
