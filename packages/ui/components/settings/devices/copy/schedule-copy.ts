import { cronWords } from "../../../../lib/cron-words";
import { formatMoment } from "../../../../lib/date";
import type {
	BotStoppedState,
	OnceFinished,
	ScheduleIdleWhy,
	ScheduleRun,
	ScheduleWhere,
} from "../../../../lib/device-management/model/schedule-where";
import type {
	ScheduleHold,
	ScheduleOutcome,
	ScheduleSkipReason,
	ServiceSchedule,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { AreaTime, DevicesT } from "../primitives/area-context";

/* A schedule on a device in words: when it runs, in which zone, and why it does not. */

export interface ScheduleText {
	expression: string;
	timezone: string;
	/** False when the event saves no zone; unknown for a schedule a device reported. */
	zoneSet?: boolean;
}

/** "At 02:00 every day"; the expression itself when no sentence fits it. */
export function scheduleWords(t: DevicesT, expression: string): string {
	return (
		cronWords(expression) ??
		t("devices:schedule.expression", "Schedule {{expression}}", { expression })
	);
}

/** The same inside a sentence: "at 02:00 every day". */
export function scheduleWordsInline(
	t: DevicesT,
	expression: string,
	locale?: string,
): string {
	const words = cronWords(expression);
	return words
		? words.charAt(0).toLocaleLowerCase(locale) + words.slice(1)
		: t("devices:schedule.expressionInline", "by the schedule {{expression}}", {
				expression,
			});
}

/** The zone a device runs the schedule in. An event without a zone runs in UTC there, never in the device's own zone. */
export function scheduleZone(
	t: DevicesT,
	schedule: Pick<ScheduleText, "timezone" | "zoneSet">,
): string {
	return schedule.zoneSet === false
		? t("devices:schedule.zoneUnset", "UTC (the event sets no time zone)")
		: schedule.timezone;
}

/** "Schedule · At 02:00 every day · Europe/Berlin". */
export function scheduleHead(t: DevicesT, schedule: ScheduleText): string {
	return t("devices:schedule.head", "Schedule · {{words}} · {{zone}}", {
		words: scheduleWords(t, schedule.expression),
		zone: scheduleZone(t, schedule),
	});
}

/** A one-time schedule as its event saves it: local date and time in its zone. */
export interface OnceText {
	date: string;
	time: string;
	timezone: string;
	zoneSet?: boolean;
}

/** "Schedule · Once on 2026-09-24 at 09:00 · Europe/Berlin": the event's own local date and time, never this computer's. */
export function scheduleOnceHead(t: DevicesT, once: OnceText): string {
	return t(
		"devices:schedule.headOnce",
		"Schedule · Once on {{date}} at {{time}} · {{zone}}",
		{ date: once.date, time: once.time, zone: scheduleZone(t, once) },
	);
}

const zoneNames = new Map<string, Intl.DateTimeFormat>();

/** "CEST", "UTC": the zone's short name at that instant. */
function shortZone(atMs: number, zone: string, locale: string): string {
	const key = `${locale}|${zone}`;
	let formatter = zoneNames.get(key);
	try {
		if (!formatter) {
			formatter = new Intl.DateTimeFormat(locale, {
				timeZone: zone,
				timeZoneName: "short",
				hour: "2-digit",
			});
			zoneNames.set(key, formatter);
		}
		return (
			formatter.formatToParts(atMs).find((part) => part.type === "timeZoneName")
				?.value ?? zone
		);
	} catch {
		return zone;
	}
}

/**
 * A scheduled time in the schedule's own zone with how far away it is:
 * "02:00 CEST · in 14 hr". The reader's local time is added only when it
 * reads differently.
 */
export function scheduleTime(
	t: DevicesT,
	atS: number,
	zone: string,
	time: Pick<AreaTime, "now" | "locale" | "ago">,
): string {
	const atMs = atS * 1000;
	const options = { now: time.now, locale: time.locale };
	let zoned: string;
	try {
		zoned = formatMoment(atMs, { ...options, timeZone: zone });
	} catch {
		// A zone this browser does not know: the reader's own time is all it can say.
		return t(
			"devices:schedule.timeOwnOnly",
			"{{own}} your time · {{relative}}",
			{
				own: formatMoment(atMs, options),
				relative: time.ago(atS, "short"),
			},
		);
	}
	const own = formatMoment(atMs, options);
	const values = {
		time: zoned,
		zone: shortZone(atMs, zone, time.locale),
		relative: time.ago(atS, "short"),
	};
	return own === zoned
		? t("devices:schedule.time", "{{time}} {{zone}} · {{relative}}", values)
		: t(
				"devices:schedule.timeOwn",
				"{{time}} {{zone}} · {{relative}} · {{own}} your time",
				{ ...values, own },
			);
}

export interface WhereNames {
	/** The device's name; "a device" without one. */
	device(deviceId: string | undefined): string;
	/** "11:00" today, "29 Sept, 11:00" otherwise. */
	at(atS: number): string;
	/** "20 min. ago". */
	ago(atS: number): string;
}

/** Names and times for `scheduleWhereText`, from the devices a view mentions and the area clock. */
export function whereNames(
	t: DevicesT,
	names: ReadonlyMap<string, string> | undefined,
	time: Pick<AreaTime, "at" | "ago">,
): WhereNames {
	return {
		device: (deviceId) =>
			(deviceId ? names?.get(deviceId) : undefined) ??
			t("devices:events.pop.aDevice", "a device"),
		at: time.at,
		ago: (atS) => time.ago(atS, "short"),
	};
}

/** Why an assigned service does not run its schedule, as the end of "… but nothing runs it: …". */
const IDLE_HOLDS: Record<ScheduleHold, (t: DevicesT) => string> = {
	other_service: (t) =>
		t(
			"devices:events.pop.hold.otherService",
			"another service of that device runs it",
		),
	hub_unreachable: (t) =>
		t(
			"devices:events.pop.hold.hubUnreachable",
			"the service is waiting for the hub to confirm it",
		),
	hub_too_old: (t) =>
		t(
			"devices:events.pop.hold.hubTooOld",
			"this hub can't hand schedules to devices yet",
		),
	not_released: (t) =>
		t(
			"devices:events.pop.hold.notReleased",
			"it was not moved to this service",
		),
	runs_elsewhere: (t) =>
		t("devices:events.pop.hold.runsElsewhere", "another service runs it"),
};

/** Why a service does not run a schedule or bot it lists, as the end of a sentence. */
export function scheduleHoldReason(
	t: DevicesT,
	hold: ScheduleHold,
	bot = false,
): string {
	return bot && hold === "hub_too_old"
		? t(
				"devices:events.pop.hold.hubTooOldBot",
				"this hub can't hand bots to devices yet",
			)
		: IDLE_HOLDS[hold](t);
}

/** What a schedule's "where it runs" sentence speaks of: a repeating schedule, a one-time schedule or a bot. */
export type WhereKind = "schedule" | "once" | "bot";

/** A bot or one-time schedule whose cloud access ended: the hub never fires it again, so nothing takes it back by itself. */
function accessEndedNoNext(
	t: DevicesT,
	where: Extract<ScheduleWhere, { fact: "device_idle" }>,
	names: WhereNames,
) {
	return t(
		"devices:events.pop.accessEndedNoNext",
		"Assigned to {{device}} › {{service}}, but its cloud access ended {{time}}. Nothing takes it back by itself: take it back here.",
		{
			device: names.device(where.deviceId),
			service: where.serviceId,
			time: where.endedAt === undefined ? "" : names.at(where.endedAt),
		},
	);
}

const BOT_STOPPED: Record<BotStoppedState, (t: DevicesT) => string> = {
	token_refused: (t) =>
		t("devices:events.pop.botIdleWhy.tokenRefused", "its token was refused"),
	intents_refused: (t) =>
		t(
			"devices:events.pop.botIdleWhy.intentsRefused",
			"Discord refused its permissions",
		),
	conflict: (t) =>
		t(
			"devices:events.pop.botIdleWhy.conflict",
			"another program uses its token",
		),
	webhook_set: (t) =>
		t(
			"devices:events.pop.botIdleWhy.webhookSet",
			"Telegram sends its messages to a webhook",
		),
};

const BOT_IDLE: Partial<Record<ScheduleIdleWhy, (t: DevicesT) => string>> = {
	stopped: (t) =>
		t("devices:events.pop.botIdleWhy.stopped", "the service is stopped"),
	failed: (t) =>
		t("devices:events.pop.botIdleWhy.failed", "the service keeps failing"),
	removed: (t) =>
		t("devices:events.pop.botIdleWhy.removed", "the service was removed"),
};

/** Why an assigned service doesn't keep its bot connected, as the end of "… but it isn't connected: …". */
function botIdleReason(
	t: DevicesT,
	where: Extract<ScheduleWhere, { fact: "device_idle" }>,
) {
	if (where.why === "not_connected")
		return BOT_STOPPED[where.botState ?? "conflict"](t);
	const plain = BOT_IDLE[where.why];
	return plain
		? plain(t)
		: scheduleHoldReason(t, where.hold ?? "hub_unreachable", true);
}

function botIdleText(
	t: DevicesT,
	where: Extract<ScheduleWhere, { fact: "device_idle" }>,
	names: WhereNames,
) {
	if (where.why === "access_ended") return accessEndedNoNext(t, where, names);
	return t(
		"devices:events.pop.botIdle",
		"Assigned to {{device}} › {{service}}, but it isn't connected: {{reason}}.",
		{
			device: names.device(where.deviceId),
			service: where.serviceId,
			reason: botIdleReason(t, where),
		},
	);
}

function idleText(
	t: DevicesT,
	where: Extract<ScheduleWhere, { fact: "device_idle" }>,
	names: WhereNames,
	kind: WhereKind,
) {
	if (kind === "bot" || where.why === "not_connected")
		return botIdleText(t, where, names);
	if (kind === "once" && where.why === "access_ended")
		return accessEndedNoNext(t, where, names);
	const values = {
		device: names.device(where.deviceId),
		service: where.serviceId,
	};
	const texts: Record<
		Exclude<ScheduleIdleWhy, "not_connected">,
		() => string
	> = {
		stopped: () =>
			t(
				"devices:events.pop.scheduleIdle.stopped",
				"Assigned to {{device}} › {{service}}, but nothing runs it: the service is stopped.",
				values,
			),
		failed: () =>
			t(
				"devices:events.pop.scheduleIdle.failed",
				"Assigned to {{device}} › {{service}}, but nothing runs it: the service keeps failing.",
				values,
			),
		held: () =>
			t(
				"devices:events.pop.scheduleIdle.held",
				"Assigned to {{device}} › {{service}}, but nothing runs it: {{hold}}.",
				{ ...values, hold: IDLE_HOLDS[where.hold ?? "hub_unreachable"](t) },
			),
		removed: () =>
			t(
				"devices:events.pop.scheduleIdle.removed",
				"Assigned to {{device}} › {{service}}, but nothing runs it: the service was removed.",
				values,
			),
		access_ended: () =>
			t(
				"devices:events.pop.scheduleIdle.accessEnded",
				"Assigned to {{device}} › {{service}}, but nothing runs it: its cloud access ended {{time}}; the hub takes it back at its next scheduled time.",
				{
					...values,
					time: where.endedAt === undefined ? "" : names.at(where.endedAt),
				},
			),
	};
	return texts[where.why]();
}

function unconfirmedText(
	t: DevicesT,
	where: Extract<ScheduleWhere, { fact: "device_unconfirmed" }>,
	names: WhereNames,
) {
	const ago = names.ago(where.seenAt);
	if (!where.deviceId)
		return t(
			"devices:events.pop.scheduleHidden",
			"Assigned to a device you can't see, not the hub. It last confirmed {{ago}}.",
			{ ago },
		);
	const device = names.device(where.deviceId);
	if (where.stale)
		return t(
			"devices:events.pop.scheduleUnconfirmed.stale",
			"Assigned to {{device}}, which has not confirmed since {{time}}. It may be off.",
			{ device, time: names.at(where.seenAt) },
		);
	// A device that is already readable is not asked to be unlocked.
	return where.readable
		? t(
				"devices:events.pop.scheduleUnconfirmed.silent",
				"Assigned to {{device}}. It last confirmed {{ago}}; its status doesn't say whether it runs.",
				{ device, ago },
			)
		: t(
				"devices:events.pop.scheduleUnconfirmed.recent",
				"Assigned to {{device}}. It last confirmed {{ago}}; unlock the device to see whether it runs.",
				{ device, ago },
			);
}

/** A one-time schedule that finished on its device: it ran, or why it didn't. Never "Runs on". */
function finishedText(
	t: DevicesT,
	where: Extract<ScheduleWhere, { fact: "device" }>,
	finished: OnceFinished,
	names: WhereNames,
) {
	const device = names.device(where.deviceId);
	const time = names.at(finished.at);
	if (finished.state === "ran")
		return t(
			"devices:events.pop.onceRan",
			"Ran on {{device}} {{time}}. Nothing runs it again.",
			{ device, time },
		);
	return finished.state === "missed"
		? t(
				"devices:events.pop.onceMissed",
				"Did not run: {{device}} wasn't running at its time ({{time}}).",
				{ device, time },
			)
		: t(
				"devices:events.pop.oncePassed",
				"Did not run: its time ({{time}}) had passed when {{service}} could first run it.",
				{ time, service: where.serviceId },
			);
}

/** Where a bot runs: the forms of the schedule sentences for something that stays connected. */
function botWhereText(t: DevicesT, where: ScheduleWhere, names: WhereNames) {
	switch (where.fact) {
		case "hub":
			return t("devices:events.pop.botHub", "No device runs it.");
		case "device":
			return t("devices:events.pop.botOn", "Connected from {{device}}.", {
				device: names.device(where.deviceId),
			});
		case "returning":
			return t(
				"devices:events.pop.botReturning",
				"Taken back from its device. Nothing runs it after {{time}}.",
				{ time: names.at(where.hubResumesAt) },
			);
		case "device_idle":
			return botIdleText(t, where, names);
		case "device_unconfirmed":
			return unconfirmedText(t, where, names);
		default:
			return where.deviceId && where.serviceId
				? t(
						"devices:events.pop.botReleased",
						"Moves to {{device}} › {{service}} when that service starts it.",
						{ device: names.device(where.deviceId), service: where.serviceId },
					)
				: t(
						"devices:events.pop.botReleasedHidden",
						"Moves to a device you can't see when its service starts it.",
					);
	}
}

/**
 * Where a schedule or bot of an online app runs, as one sentence. It says
 * "Runs on {device}" only when a running service is known to run it, and a
 * one-time schedule that finished says what happened instead.
 */
export function scheduleWhereText(
	t: DevicesT,
	where: ScheduleWhere,
	names: WhereNames,
	kind: WhereKind = "schedule",
): string {
	if (kind === "bot") return botWhereText(t, where, names);
	if (where.fact === "hub")
		return kind === "once"
			? t("devices:events.pop.onceHub", "Not on a device: the hub keeps it.")
			: t("devices:events.pop.scheduleHub", "Runs on the hub.");
	if (where.fact === "device")
		return where.finished
			? finishedText(t, where, where.finished, names)
			: t(
					"devices:events.pop.scheduleOn",
					"Runs on {{device}}, not on the hub.",
					{ device: names.device(where.deviceId) },
				);
	if (where.fact === "returning")
		return t(
			"devices:events.pop.scheduleReturning",
			"Returns to the hub at {{time}}.",
			{ time: names.at(where.hubResumesAt) },
		);
	if (where.fact === "device_idle") return idleText(t, where, names, kind);
	if (where.fact === "device_unconfirmed")
		return unconfirmedText(t, where, names);
	return where.deviceId && where.serviceId
		? t(
				"devices:events.pop.scheduleReleased",
				"Moves to {{device}} › {{service}} when that service starts it. The hub runs it until then.",
				{ device: names.device(where.deviceId), service: where.serviceId },
			)
		: t(
				"devices:events.pop.scheduleReleasedHidden",
				"Moves to a device you can't see when its service starts it. The hub runs it until then.",
			);
}

/** A service row's line for its schedules: "Runs on a schedule · next …", without the time when no next run is known. */
export function scheduledLine(
	t: DevicesT,
	next: { at: number; zone: string } | null,
	time: Pick<AreaTime, "now" | "locale" | "ago">,
): string {
	return next
		? t(
				"devices:app.device.scheduledNext",
				"Runs on a schedule · next {{time}}",
				{
					time: scheduleTime(t, next.at, next.zone, time),
				},
			)
		: t("devices:app.device.scheduled", "Runs on a schedule");
}

/** Schedules by name as one list ("Nightly report and Weekly digest"); an event whose name isn't known keeps its id. */
export function scheduleNameList(
	eventIds: readonly string[],
	names: ReadonlyMap<string, string>,
	locale: string,
): string {
	return new Intl.ListFormat(locale, { type: "conjunction" }).format(
		eventIds.map((eventId) => names.get(eventId) ?? eventId),
	);
}

/** A schedule that someone can hand back to the hub: it is released to a service or assigned to one. */
export function canRunOnHubAgain(where: ScheduleWhere | undefined): boolean {
	return (
		where?.fact === "released" ||
		where?.fact === "device" ||
		where?.fact === "device_idle" ||
		where?.fact === "device_unconfirmed"
	);
}

export interface HoldNames {
	/** The other service of this device that runs the schedule. */
	service?: string;
	/** Where it runs instead: "{device} › {service}", or "another service". */
	where?: string;
}

const HOLDS: Record<ScheduleHold, (t: DevicesT, names: HoldNames) => string> = {
	not_released: (t) =>
		t(
			"devices:serviceStatus.schedules.hold.notReleased",
			"Not running here: nobody who can edit this app's events has moved it to this service. The hub runs it.",
		),
	runs_elsewhere: (t, { where }) =>
		t(
			"devices:serviceStatus.schedules.hold.runsElsewhere",
			"Not running here: {{where}} runs it. A schedule runs in one place.",
			{
				where:
					where ??
					t(
						"devices:serviceStatus.schedules.hold.anotherService",
						"another service",
					),
			},
		),
	other_service: (t, { service }) =>
		t(
			"devices:serviceStatus.schedules.hold.otherService",
			"Not running here: {{service}} on this device runs it.",
			{
				service:
					service ??
					t(
						"devices:serviceStatus.schedules.hold.anotherService",
						"another service",
					),
			},
		),
	hub_unreachable: (t) =>
		t(
			"devices:serviceStatus.schedules.hold.hubUnreachable",
			"Waiting for the hub to confirm that it stopped running it.",
		),
	hub_too_old: (t) =>
		t(
			"devices:serviceStatus.schedules.hold.hubTooOld",
			"This hub can't hand schedules to devices yet. Update the hub.",
		),
};

/** Why a running service does not run one of its schedules. */
export function scheduleHoldText(
	t: DevicesT,
	hold: ScheduleHold,
	names: HoldNames = {},
): string {
	return HOLDS[hold](t, names);
}

const OUTCOMES: Record<ScheduleOutcome, (t: DevicesT) => string> = {
	succeeded: (t) => t("devices:schedule.outcome.succeeded", "succeeded"),
	failed: (t) => t("devices:schedule.outcome.failed", "failed"),
	cancelled: (t) => t("devices:schedule.outcome.cancelled", "cut off"),
	timed_out: (t) =>
		t("devices:schedule.outcome.timedOut", "stopped after 24 hours"),
};

/** How a scheduled run ended. */
export function scheduleOutcome(t: DevicesT, outcome: ScheduleOutcome): string {
	return OUTCOMES[outcome](t);
}

const SKIPS: Record<ScheduleSkipReason, (t: DevicesT) => string> = {
	overlap: (t) =>
		t("devices:schedule.skip.overlap", "the previous run was still going"),
	missed: (t) =>
		t(
			"devices:schedule.skip.missed",
			"the device was off, asleep or restarting at its time",
		),
	busy: (t) =>
		t("devices:schedule.skip.busy", "8 scheduled runs were already going"),
};

/** Why the device skipped a scheduled time. */
export function scheduleSkip(t: DevicesT, reason: ScheduleSkipReason): string {
	return SKIPS[reason](t);
}

export interface ScheduleRunNames extends HoldNames {
	device: string;
	/** The service whose schedule is described. */
	own?: string;
}

export interface ScheduleRunNamesInput {
	/** The service whose schedule is described, and the name of its device. */
	deviceId: string;
	serviceId: string;
	device: string;
	eventId: string;
	/** Where the hub says the schedule runs (online apps). */
	where?: ScheduleWhere;
	deviceName?(deviceId: string): string;
	/** The device's services, to name the one that runs the schedule instead. */
	siblings?: readonly Pick<ServiceView, "serviceId" | "schedules">[];
}

/** The names a hold sentence needs: the other service of this device, or the place the hub assigned the schedule to. */
export function scheduleRunNames(
	t: DevicesT,
	input: ScheduleRunNamesInput,
): ScheduleRunNames {
	const { where, serviceId, deviceId } = input;
	const sibling = input.siblings?.find(
		(view) =>
			view.serviceId !== serviceId &&
			Array.isArray(view.schedules) &&
			view.schedules.some(
				(entry) => entry.event_id === input.eventId && !entry.hold,
			),
	);
	const assigned =
		where && "deviceId" in where && where.deviceId && where.serviceId
			? { deviceId: where.deviceId, serviceId: where.serviceId }
			: null;
	const elsewhere =
		assigned &&
		!(assigned.deviceId === deviceId && assigned.serviceId === serviceId)
			? t("devices:schedule.place", "{{device}} › {{service}}", {
					device: input.deviceName?.(assigned.deviceId) ?? assigned.deviceId,
					service: assigned.serviceId,
				})
			: undefined;
	return {
		device: input.device,
		own: serviceId,
		...(sibling ? { service: sibling.serviceId } : {}),
		...(elsewhere ? { where: elsewhere } : {}),
	};
}

type RunTime = Pick<AreaTime, "now" | "locale" | "ago">;

/** "Last run 02:00 CEST · 10 hr ago · succeeded"; without a time (a status snapshot) only the result. */
function lastRunText(t: DevicesT, entry: ServiceSchedule, time: RunTime) {
	const outcome = entry.last_outcome
		? scheduleOutcome(t, entry.last_outcome)
		: null;
	if (typeof entry.last_at === "number")
		return outcome
			? t(
					"devices:serviceStatus.schedules.last",
					"Last run {{time}} · {{outcome}}",
					{
						time: scheduleTime(t, entry.last_at, entry.timezone, time),
						outcome,
					},
				)
			: t("devices:serviceStatus.schedules.lastRunning", "Last run {{time}}", {
					time: scheduleTime(t, entry.last_at, entry.timezone, time),
				});
	return outcome
		? t("devices:serviceStatus.schedules.lastOutcome", "Last run {{outcome}}", {
				outcome,
			})
		: null;
}

/** Why an armed schedule shows no next run: the status is too old to say, or nothing is due. */
function noNextText(
	t: DevicesT,
	run: Extract<ScheduleRun, { state: "armed" }>,
) {
	return run.stale
		? t(
				"devices:serviceStatus.schedules.stale",
				"This status is too old to say when it runs next.",
			)
		: t(
				"devices:serviceStatus.schedules.noNext",
				"No further run is scheduled.",
			);
}

/** A one-time schedule that is still to come on this service: when it runs, or that a run is going. */
function onceLines(
	t: DevicesT,
	entry: ServiceSchedule & { once_at: number },
	time: RunTime,
) {
	return [
		entry.running || entry.once_state === "started"
			? t("devices:serviceStatus.schedules.running", "A run is going now.")
			: t(
					"devices:serviceStatus.schedules.once.willRun",
					"Runs once {{time}}",
					{
						time: scheduleTime(t, entry.once_at, entry.timezone, time),
					},
				),
	];
}

/** How a one-time schedule that ran ended: cut off, with its outcome, or just when. */
function ranLine(t: DevicesT, finished: OnceFinished, when: string) {
	const outcome = finished.outcome;
	if (outcome === "cancelled")
		return t(
			"devices:serviceStatus.schedules.once.cutOff",
			"Was cut off {{time}}; it isn't started again.",
			{ time: when },
		);
	return outcome
		? t(
				"devices:serviceStatus.schedules.once.ran",
				"Ran {{time}} · {{outcome}}",
				{
					time: when,
					outcome: scheduleOutcome(t, outcome),
				},
			)
		: t("devices:serviceStatus.schedules.once.ranAt", "Ran {{time}}", {
				time: when,
			});
}

/** A one-time schedule that finished: how, and that nothing runs it again. */
function finishedLines(
	t: DevicesT,
	run: Extract<ScheduleRun, { state: "finished" }>,
	names: ScheduleRunNames,
	time: RunTime,
) {
	const { entry, finished } = run;
	const when = scheduleTime(t, finished.at, entry.timezone, time);
	const service =
		names.own ??
		t("devices:serviceStatus.schedules.once.thisService", "this service");
	const lines = {
		ran: () => ranLine(t, finished, when),
		missed: () =>
			t(
				"devices:serviceStatus.schedules.once.missed",
				"Missed: {{device}} wasn't running at its time ({{time}}).",
				{ device: names.device, time: when },
			),
		passed: () =>
			t(
				"devices:serviceStatus.schedules.once.passed",
				"Its time ({{time}}) had passed when {{service}} could first run it.",
				{ time: when, service },
			),
	} satisfies Record<OnceFinished["state"], () => string>;
	return [
		lines[finished.state](),
		t("devices:serviceStatus.schedules.once.done", "Nothing more to run."),
	];
}

function armedLines(
	t: DevicesT,
	run: Extract<ScheduleRun, { state: "armed" }>,
	names: ScheduleRunNames,
	time: RunTime,
	sayNoNext: boolean,
) {
	const { entry, next } = run;
	if (entry.once_at !== undefined)
		return onceLines(t, { ...entry, once_at: entry.once_at }, time);
	const when = next ? scheduleTime(t, next.at, entry.timezone, time) : null;
	const nextLine =
		when === null
			? sayNoNext
				? noNextText(t, run)
				: null
			: entry.clock_behind
				? t(
						"devices:serviceStatus.schedules.clockBehind",
						"Waiting until {{time}}: {{device}}'s clock is behind its last run.",
						{ time: when, device: names.device },
					)
				: next?.computed
					? t(
							"devices:serviceStatus.schedules.nextComputed",
							"Next run by its schedule: {{time}}",
							{ time: when },
						)
					: t("devices:serviceStatus.schedules.next", "Next run {{time}}", {
							time: when,
						});
	return [
		entry.running
			? t("devices:serviceStatus.schedules.running", "A run is going now.")
			: null,
		nextLine,
		lastRunText(t, entry, time),
	].filter((line): line is string => line !== null);
}

/**
 * What one schedule of a service does now, as sentences. A next run and
 * counters are said only for a schedule that is armed on a service that runs;
 * what is not known reads as not known, never as "never ran".
 */
export function scheduleRunLines(
	t: DevicesT,
	run: ScheduleRun,
	names: ScheduleRunNames,
	time: RunTime,
	/** `noNext`: an armed schedule without a next run says why (the service's own page; a table cell stays short). */
	options: { noNext?: boolean } = {},
): string[] {
	if (run.state === "armed")
		return armedLines(t, run, names, time, options.noNext === true);
	if (run.state === "finished") return finishedLines(t, run, names, time);
	if (run.state === "held") return [scheduleHoldText(t, run.hold, names)];
	if (run.state === "stopped")
		return [
			t(
				"devices:serviceStatus.schedules.stopped",
				"No runs while the service is not running.",
			),
		];
	if (run.state === "not_reported")
		return [
			t("devices:serviceStatus.schedules.notReported", "Not reported yet."),
		];
	if (run.state === "needs_agent")
		return [
			t(
				"devices:serviceStatus.schedules.needsAgent",
				"Update the device agent to see its schedules.",
			),
		];
	return [
		t(
			"devices:serviceStatus.schedules.unknown",
			"Not in this status. Connect live to see its runs.",
		),
	];
}
