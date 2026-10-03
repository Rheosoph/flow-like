"use client";

import { useTranslation } from "@flow-like/locales";
import { Bot, SlidersHorizontal } from "lucide-react";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import { whereOf } from "../../../../lib/device-management/model/schedule-where";
import type {
	DeviceViewModel,
	ScheduleHold,
	ServiceBot,
	ServiceBotProvider,
	ServiceBotState,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { eventTypeLabel } from "../copy/eligibility-copy";
import { scheduleOutcome, scheduleRunNames } from "../copy/schedule-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { RunItHere } from "./schedules-block";
import {
	type ListRun,
	appEventRows,
	countersFresh,
	listRun,
	reportedEntries,
	serviceEventIds,
} from "./service-events";
import type { ServiceApp } from "./service-header";

/* Service › Status: the service's Telegram and Discord bots, whether each is connected, and why one is not. */

export interface BotRow {
	eventId: string;
	/** The event's name; undefined when the app no longer has it, or its names can't be read. */
	name?: string;
	provider: ServiceBotProvider;
}

const PROVIDER: Readonly<Record<string, ServiceBotProvider>> = {
	telegram: "telegram",
	discord: "discord",
};

/** The bots of a service: what its process reported, and the events it serves that the app lists as a bot. */
export function botRows(
	service: Pick<ServiceView, "bots" | "events">,
	app: Pick<ServiceApp, "view">,
): BotRow[] {
	const known = appEventRows(app);
	const reported = reportedEntries(service.bots);
	const isBot = (eventId: string) =>
		known.get(eventId)?.eligibility.kind === "bot";
	const providerOf = (eventId: string) => {
		const same = (value: ServiceBot) => value.event_id === eventId;
		const entry = reported.find(same);
		return entry?.provider ?? PROVIDER[known.get(eventId)?.eventType ?? ""];
	};
	return serviceEventIds(service, reported, isBot).flatMap((eventId) => {
		const provider = providerOf(eventId);
		const name = known.get(eventId)?.name;
		return provider ? [{ eventId, ...(name ? { name } : {}), provider }] : [];
	});
}

interface BotNames {
	service: string;
	/** Another service of this device that runs it. */
	sibling?: string;
	/** "{device} › {service}" where the hub says it runs instead. */
	where?: string;
}

const anotherService = (t: DevicesT) =>
	t(
		"devices:serviceStatus.bots.hold.anotherService",
		"Not connected: another service runs it. A bot runs in one place.",
	);

/** Why a running service does not connect a bot it holds. */
const HOLD_LINES: Record<
	ScheduleHold,
	(t: DevicesT, names: BotNames) => string
> = {
	not_released: (t) =>
		t(
			"devices:serviceStatus.bots.hold.notReleased",
			"Not connected: nobody who can edit this app's events has moved it to this service.",
		),
	other_service: (t, { sibling }) =>
		sibling
			? t(
					"devices:serviceStatus.bots.hold.otherService",
					"Not connected: {{service}} on this device runs it. A bot runs in one place.",
					{ service: sibling },
				)
			: anotherService(t),
	runs_elsewhere: (t, { where }) =>
		where
			? t(
					"devices:serviceStatus.bots.hold.runsElsewhere",
					"Not connected: {{where}} runs it. A bot runs in one place.",
					{ where },
				)
			: anotherService(t),
	hub_unreachable: (t) =>
		t(
			"devices:serviceStatus.bots.hold.hubUnreachable",
			"Waiting for the hub to confirm that this service may run it.",
		),
	hub_too_old: (t) =>
		t(
			"devices:serviceStatus.bots.hold.hubTooOld",
			"This hub can't hand bots to devices yet. Update the hub.",
		),
};

function connectedLine(t: DevicesT, entry: ServiceBot, time: AreaTime) {
	const at = entry.connected_at;
	const since = typeof at === "number" ? time.at(at) : null;
	const name = entry.bot_name;
	if (name && since)
		return t(
			"devices:serviceStatus.bots.connectedAs",
			"Connected as {{name}} since {{time}}.",
			{ name, time: since },
		);
	if (name)
		return t(
			"devices:serviceStatus.bots.connectedAsNow",
			"Connected as {{name}}.",
			{ name },
		);
	return since
		? t(
				"devices:serviceStatus.bots.connectedSince",
				"Connected since {{time}}.",
				{ time: since },
			)
		: t("devices:serviceStatus.bots.connected", "Connected.");
}

type StateLine = (
	t: DevicesT,
	entry: ServiceBot,
	names: BotNames,
	time: AreaTime,
) => string;

/** What a bot that may connect does now, by the state its process reported. */
const STATE_LINES: Record<ServiceBotState, StateLine> = {
	connected: (t, entry, _names, time) => connectedLine(t, entry, time),
	ok: (t) =>
		t(
			"devices:serviceStatus.bots.ok",
			"Connected or reconnecting when this status was published.",
		),
	connecting: (t) => t("devices:serviceStatus.bots.connecting", "Connecting."),
	reconnecting: (t) =>
		t("devices:serviceStatus.bots.reconnecting", "Reconnecting."),
	token_refused: (t, entry) =>
		entry.provider === "telegram"
			? t(
					"devices:serviceStatus.bots.tokenRefused.telegram",
					"Telegram refused the token. Enter a new one under Configuration.",
				)
			: t(
					"devices:serviceStatus.bots.tokenRefused.discord",
					"Discord refused the token. Enter a new one under Configuration.",
				),
	intents_refused: (t, _entry, names) =>
		t(
			"devices:serviceStatus.bots.intentsRefused",
			"Discord refused the bot's permissions. Turn on the message content intent in the Discord Developer Portal, then restart {{service}}.",
			{ service: names.service },
		),
	conflict: (t) =>
		t(
			"devices:serviceStatus.bots.conflict",
			"Another program uses this bot's token. A bot runs in one place: stop it there.",
		),
	webhook_set: (t, _entry, names) =>
		t(
			"devices:serviceStatus.bots.webhookSet",
			"Telegram sends this bot's messages to a webhook. Remove it in Events, then restart {{service}}.",
			{ service: names.service },
		),
	waiting: (t, _entry, names) =>
		t(
			"devices:serviceStatus.bots.waiting",
			"Not connected yet: {{service}} is checking whether it may run it.",
			{ service: names.service },
		),
};

type Unreported = Exclude<ListRun<ServiceBot>["state"], "reported">;

/** One sentence for a bot whose process can't say anything about it. */
const UNREPORTED_LINES: Record<Unreported, (t: DevicesT) => string> = {
	stopped: (t) =>
		t(
			"devices:serviceStatus.bots.stopped",
			"Not connected while the service is not running.",
		),
	not_reported: (t) =>
		t("devices:serviceStatus.bots.notReported", "Not reported yet."),
	needs_agent: (t) =>
		t(
			"devices:serviceStatus.bots.needsAgent",
			"Update the device agent to see its bots.",
		),
	unknown: (t) =>
		t(
			"devices:serviceStatus.bots.unknown",
			"Not in this status. Connect live to see whether it is connected.",
		),
};

function lastMessageLine(t: DevicesT, entry: ServiceBot, time: AreaTime) {
	const at = entry.last_message_at;
	if (typeof at !== "number") return null;
	const when = time.ago(at, "short");
	return entry.last_outcome
		? t(
				"devices:serviceStatus.bots.lastMessageRun",
				"Last message {{time}} · its run {{outcome}}",
				{ time: when, outcome: scheduleOutcome(t, entry.last_outcome) },
			)
		: t("devices:serviceStatus.bots.lastMessage", "Last message {{time}}", {
				time: when,
			});
}

function runsLine(t: DevicesT, entry: ServiceBot) {
	if (typeof entry.runs !== "number") return null;
	return t("devices:serviceStatus.bots.runs", {
		count: entry.runs_today ?? 0,
		total: entry.runs,
		failed: entry.failed ?? 0,
		defaultValue_one:
			"{{count, number}} run today · {{total, number}} since it started · {{failed, number}} failed",
		defaultValue_other:
			"{{count, number}} runs today · {{total, number}} since it started · {{failed, number}} failed",
	});
}

function runningLine(t: DevicesT, entry: ServiceBot) {
	const running = entry.running ?? 0;
	return running > 0
		? t("devices:serviceStatus.bots.running", {
				count: running,
				defaultValue_one: "{{count, number}} run is going now.",
				defaultValue_other: "{{count, number}} runs are going now.",
			})
		: null;
}

function droppedLine(t: DevicesT, entry: ServiceBot) {
	const dropped = entry.dropped ?? 0;
	return dropped > 0
		? t("devices:serviceStatus.bots.dropped", {
				count: dropped,
				defaultValue_one:
					"Not answered: {{count, number}} message (too many at once, or too old after a start).",
				defaultValue_other:
					"Not answered: {{count, number}} messages (too many at once, or too old after a start).",
			})
		: null;
}

const isLine = (line: string | null): line is string => line !== null;

/** The numbers a live row carries: last message, runs, messages it didn't answer. */
function counterLines(t: DevicesT, entry: ServiceBot, time: AreaTime) {
	return [
		lastMessageLine(t, entry, time),
		runsLine(t, entry),
		runningLine(t, entry),
		droppedLine(t, entry),
	].filter(isLine);
}

function botLines(
	t: DevicesT,
	run: ListRun<ServiceBot>,
	names: BotNames,
	time: AreaTime,
	fresh: boolean,
) {
	if (run.state !== "reported") return [UNREPORTED_LINES[run.state](t)];
	const { entry } = run;
	if (entry.hold) return [HOLD_LINES[entry.hold](t, names)];
	return [
		STATE_LINES[entry.state](t, entry, names, time),
		...(fresh ? counterLines(t, entry, time) : []),
	];
}

/** The service on this device that runs the bot instead, when this one holds it for that reason. */
function siblingOf(
	device: DeviceViewModel,
	service: ServiceView,
	eventId: string,
) {
	if (!Array.isArray(device.services)) return undefined;
	const runsIt = (entry: ServiceBot) =>
		entry.event_id === eventId && entry.hold === null;
	const other = (view: ServiceView) =>
		view.serviceId !== service.serviceId &&
		reportedEntries(view.bots).some(runsIt);
	return device.services.find(other)?.serviceId;
}

/** The names the hold sentences need: where the hub says it runs instead, and the other service of this device. */
function useBotNames(
	device: DeviceViewModel,
	service: ServiceView,
	app: ServiceApp,
	eventId: string,
): BotNames {
	const { t } = useTranslation("devices");
	const groups = new Map(
		(app.view?.groups ?? []).map((group) => [group.deviceId, group.name]),
	);
	const deviceLabel = (id: string) => groups.get(id) ?? id;
	const { where } = scheduleRunNames(t, {
		deviceId: service.deviceId,
		serviceId: service.serviceId,
		device: deviceName(device.row),
		eventId,
		...(app.view ? { where: whereOf(app.view.schedules, eventId) } : {}),
		deviceName: deviceLabel,
	});
	const sibling = siblingOf(device, service, eventId);
	return {
		service: service.serviceId,
		...(sibling ? { sibling } : {}),
		...(where ? { where } : {}),
	};
}

/** What the row says about its state, for tests and styles: a reported state, `held`, or why nothing is reported. */
function stateOf(locked: boolean, run: ListRun<ServiceBot>) {
	if (locked) return "locked";
	if (run.state !== "reported") return run.state;
	return run.entry.hold ? "held" : run.entry.state;
}

function BotItem({
	row,
	device,
	service,
	app,
}: Readonly<{
	row: BotRow;
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const names = useBotNames(device, service, app, row.eventId);
	const locked = service.freshness.age === "locked";
	const run = listRun(service, service.bots, row.eventId);
	const name = row.name ?? t("service.serves.removed", "A removed event");
	const lines = locked
		? [t("view.matrix.locked", "Unknown until unlocked")]
		: botLines(t, run, names, time, countersFresh(service));
	const state = stateOf(locked, run);
	return (
		<li
			data-bot={row.eventId}
			data-bot-state={state}
			className="flex min-w-0 flex-col gap-1 border-t border-hairline py-2.5 text-ui first:border-t-0 first:pt-0 last:pb-0"
		>
			<p className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
				<b className="font-semibold">{name}</b>
				<span className="text-muted-foreground">
					{eventTypeLabel(t, row.provider)}
				</span>
			</p>
			{lines.map((line) => (
				<p key={line} data-bot-line="" className="text-ink-2">
					{line}
				</p>
			))}
			{state === "held" &&
			run.state === "reported" &&
			run.entry.hold === "not_released" &&
			app.appId ? (
				<RunItHere
					target={{
						appId: app.appId,
						eventId: row.eventId,
						event: name,
						deviceId: service.deviceId,
						serviceId: service.serviceId,
						bot: true,
					}}
				/>
			) : null}
			{state === "token_refused" ? (
				<DvButton asChild size="xs" icon={SlidersHorizontal} className="w-fit">
					<a
						{...link({
							screen: "service",
							deviceId: service.deviceId,
							serviceId: service.serviceId,
							tab: "configuration",
						})}
					>
						{t("serviceStatus.bots.openConfiguration", "Open Configuration")}
					</a>
				</DvButton>
			) : null}
		</li>
	);
}

/**
 * The bots of the service with what each does now: connected, held and why,
 * or stopped by a refusal with what fixes it. What is not known reads as not
 * known, never as "not connected".
 */
export function BotsBlock({
	device,
	service,
	app,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
}>) {
	const { t } = useTranslation("devices");
	const rows = botRows(service, app);
	if (!rows.length) return null;
	return (
		<Block
			id="service-bots"
			icon={Bot}
			title={t("serviceStatus.bots.title", "Bots")}
			count={rows.length}
			stamp={<FreshnessStamp {...stampOf(service.freshness)} />}
			foot={t(
				"serviceStatus.bots.rule",
				"A bot runs in one place. It connects once this service may run it, and stays connected while the service runs.",
			)}
		>
			<ul className="flex min-w-0 flex-col">
				{rows.map((row) => (
					<BotItem
						key={row.eventId}
						row={row}
						device={device}
						service={service}
						app={app}
					/>
				))}
			</ul>
			{service.botsTruncated ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceStatus.bots.truncated",
						"The device reported only some of this service's bots.",
					)}
				</p>
			) : null}
		</Block>
	);
}
