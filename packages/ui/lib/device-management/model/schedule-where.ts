import { nextRuns } from "../schedule";
import type {
	AgeState,
	AppDevicePlacements,
	AppScheduleRow,
	Freshness,
	ScheduleHold,
	ScheduleOutcome,
	ServiceBot,
	ServiceBotState,
	ServiceSchedule,
	ServiceView,
} from "./types";

/*
 * Where one claimed event of an online app runs (a schedule of either kind, a
 * bot), from what the hub lists, the approval of the service it is assigned
 * to and what is known of that device. The hub's row alone never says
 * "runs": it keeps saying `device` while the service is stopped, failing or
 * gone, and after a one-time schedule finished.
 */

/** A device's claim confirmation is this old after four missed confirmations (one every 30 minutes). */
export const SCHEDULE_CONFIRM_STALE_S = 2 * 3_600;

export type ScheduleIdleWhy =
	| "stopped"
	| "failed"
	| "held"
	| "removed"
	| "access_ended"
	/** A bot whose provider refused it or that another program uses: see `botState`. */
	| "not_connected";

/** The bot states in which nothing answers its messages until someone acts. */
export const BOT_STOPPED_STATES = [
	"token_refused",
	"intents_refused",
	"conflict",
	"webhook_set",
] as const satisfies readonly ServiceBotState[];
export type BotStoppedState = (typeof BOT_STOPPED_STATES)[number];

const isStoppedBot = (state: ServiceBotState): state is BotStoppedState =>
	(BOT_STOPPED_STATES as readonly ServiceBotState[]).includes(state);

/** A one-time schedule that finished on its device: nothing runs it again there. */
export interface OnceFinished {
	state: "ran" | "missed" | "passed";
	/** `ran`: when it ran (its time when the status doesn't say); otherwise its time. */
	at: number;
	/** `ran`: how the run ended, when the status says. */
	outcome?: ScheduleOutcome | null;
}

interface Assigned {
	deviceId: string;
	serviceId: string;
	since: number;
	seenAt: number;
}

export type ScheduleWhere =
	/** The hub runs it and nobody released it; for a bot: no device runs it. */
	| { fact: "hub" }
	/** Released to a service that has not started it: the hub runs it until then. */
	| {
			fact: "released";
			since: number;
			deviceId?: string;
			serviceId?: string;
			hubResumesAt?: number;
	  }
	/**
	 * A running service is known to run it: its schedule entry or its bot
	 * entry. `live`: the entry carries times and counters. `finished`: a
	 * one-time schedule that ran, was missed or whose time had passed; it never
	 * runs again, also when its service is stopped.
	 */
	| ({
			fact: "device";
			schedule?: ServiceSchedule;
			bot?: ServiceBot;
			live: boolean;
			finished?: OnceFinished;
	  } & Assigned)
	/** Assigned to a service, and it is known that nothing runs it. */
	| ({
			fact: "device_idle";
			why: ScheduleIdleWhy;
			hold?: ScheduleHold;
			/** `access_ended`: when the cloud access ended. */
			endedAt?: number;
			/** `not_connected`: what keeps the bot off. */
			botState?: BotStoppedState;
	  } & Assigned)
	/** Assigned to a service whose state is not known (locked, no fresh status, or a device the viewer can't see). */
	| {
			fact: "device_unconfirmed";
			since: number;
			seenAt: number;
			/** The device has not confirmed for two hours: it may be off. */
			stale: boolean;
			/** The device's services can be read, and they don't say whether it runs (a status too old, or a service that has not said yet). */
			readable: boolean;
			deviceId?: string;
			serviceId?: string;
	  }
	/** Handed back; the hub runs it again at `hubResumesAt`. */
	| { fact: "returning"; hubResumesAt: number };

export interface ScheduleWhereDevice {
	id: string;
	/** The device's readable services; anything else means its state is not known. */
	services: readonly ServiceView[] | object;
}

export interface ScheduleWhereInput {
	placements: Pick<AppDevicePlacements, "placements">;
	devices: readonly ScheduleWhereDevice[];
	/** Hub-corrected unix seconds. */
	now: number;
}

const FRESH_AGES: readonly AgeState[] = ["live", "current", "delayed"];
const isFresh = (freshness: Freshness) => FRESH_AGES.includes(freshness.age);

/** A one-time entry that ran, was missed or whose time had passed; undefined for anything else. */
export function onceFinished(entry: ServiceSchedule): OnceFinished | undefined {
	const state = entry.once_state;
	if (entry.once_at === undefined) return undefined;
	if (state === "missed" || state === "passed")
		return { state, at: entry.once_at };
	if (state !== "ran") return undefined;
	return {
		state,
		at: typeof entry.last_at === "number" ? entry.last_at : entry.once_at,
		...(entry.last_outcome === undefined
			? {}
			: { outcome: entry.last_outcome }),
	};
}

/**
 * What one schedule of a service does now. A next run exists only for a
 * schedule that is armed on a service that runs: the device's own while the
 * row is live, computed from the schedule on a status that is not stale,
 * none otherwise. A finished one-time schedule is reported also while its
 * service is stopped.
 */
export type ScheduleRun =
	| {
			state: "armed";
			entry: ServiceSchedule;
			next: { at: number; computed: boolean } | null;
			/** The status is too old to say when it runs next: `next` is null for that reason, not because nothing is due. */
			stale?: true;
	  }
	| { state: "held"; entry: ServiceSchedule; hold: ScheduleHold }
	/** A one-time schedule that will never run again on this service. */
	| { state: "finished"; entry: ServiceSchedule; finished: OnceFinished }
	/** The service is not running: no runs, and no numbers of a process that is gone. */
	| { state: "stopped" }
	/** The service runs and has not said anything about this schedule yet. */
	| { state: "not_reported" }
	/** The device's agent is too old to say. */
	| { state: "needs_agent" }
	/** This plane does not say (locked, or a status without the agent's flags). */
	| { state: "unknown" };

function nextRun(
	entry: ServiceSchedule,
	freshness: Freshness,
	nowS: number,
): { at: number; computed: boolean } | null {
	if (freshness.age === "live" && typeof entry.next_at === "number")
		return { at: entry.next_at, computed: false };
	// The device said "none" (null), or the row is too old to compute from.
	if (entry.next_at === null || !isFresh(freshness)) return null;
	if (entry.once_at !== undefined) return { at: entry.once_at, computed: true };
	if (entry.expression === undefined) return null;
	const [next] = nextRuns(
		{ expression: entry.expression, timezone: entry.timezone },
		nowS * 1_000,
		1,
	);
	return next === undefined
		? null
		: { at: Math.floor(next / 1_000), computed: true };
}

const reportedSchedules = (view: Pick<ServiceView, "schedules">) =>
	Array.isArray(view.schedules) ? view.schedules : [];

const reportedBots = (view: Pick<ServiceView, "bots">) =>
	Array.isArray(view.bots) ? view.bots : [];

export function scheduleRun(
	view: Pick<ServiceView, "schedules" | "freshness" | "desired" | "conv">,
	eventId: string,
	nowS: number,
): ScheduleRun {
	const { schedules } = view;
	if (schedules === undefined || schedules === "not_loaded")
		return { state: "unknown" };
	if (schedules === "needs_agent") return { state: "needs_agent" };
	const entry = reportedSchedules(view).find(
		(value) => value.event_id === eventId,
	);
	const finished = entry && onceFinished(entry);
	if (entry && finished) return { state: "finished", entry, finished };
	// A stopped or failed service runs nothing, whatever its row still carries.
	if (idleWhy(view) !== null) return { state: "stopped" };
	// A service that runs and has no entry has not armed the schedule yet.
	if (!entry) return { state: "not_reported" };
	if (entry.hold) return { state: "held", entry, hold: entry.hold };
	return {
		state: "armed",
		entry,
		next: nextRun(entry, view.freshness, nowS),
		...(isFresh(view.freshness) ? {} : { stale: true as const }),
	};
}

/** What one bot of a service does now, from its status row. */
export type BotRun =
	| { state: "entry"; bot: ServiceBot }
	/** The service is stopped or failing: nothing connects. */
	| { state: "stopped" }
	/** The service runs and has not said anything about this bot yet. */
	| { state: "not_reported" }
	/** The device's agent is too old to say. */
	| { state: "needs_agent" }
	/** This plane does not say (locked, or a status without the agent's flags). */
	| { state: "unknown" };

export function botRun(
	view: Pick<ServiceView, "bots" | "desired" | "conv">,
	eventId: string,
): BotRun {
	const { bots } = view;
	if (bots === undefined || bots === "not_loaded") return { state: "unknown" };
	if (bots === "needs_agent") return { state: "needs_agent" };
	if (idleWhy(view) !== null) return { state: "stopped" };
	const bot = reportedBots(view).find((entry) => entry.event_id === eventId);
	return bot ? { state: "entry", bot } : { state: "not_reported" };
}

/**
 * The schedules of a service by event id: what its process reported, and the
 * events it serves that the app lists as a schedule. A service that is
 * stopped, or whose agent can't say, still lists the ones its events name.
 */
export function serviceScheduleIds(
	service: Pick<ServiceView, "schedules" | "events">,
	isSchedule: (eventId: string) => boolean,
): string[] {
	return [
		...new Set([
			...(service.events ?? [])
				.map((event) => event.event_id)
				.filter(isSchedule),
			...reportedSchedules(service).map((entry) => entry.event_id),
		]),
	];
}

/** The earliest next run among these schedules of a service, with its zone; null when none of them has one. */
export function nextScheduledRun(
	view: Pick<ServiceView, "schedules" | "freshness" | "desired" | "conv">,
	eventIds: readonly string[],
	nowS: number,
): { at: number; zone: string } | null {
	return (
		eventIds
			.map((eventId) => scheduleRun(view, eventId, nowS))
			.flatMap((run) =>
				run.state === "armed" && run.next
					? [{ at: run.next.at, zone: run.entry.timezone }]
					: [],
			)
			.sort((a, b) => a.at - b.at)[0] ?? null
	);
}

/**
 * A service that runs is known to run this schedule or bot: it reports it,
 * nothing holds it, a one-time schedule has not finished and a bot is not
 * kept off by its provider.
 */
export function runsSchedule(
	view: Pick<ServiceView, "schedules" | "bots" | "desired" | "conv">,
	eventId: string,
): boolean {
	if (idleWhy(view) !== null) return false;
	const entry = reportedSchedules(view).find(
		(value) => value.event_id === eventId,
	);
	if (entry) return entry.hold === null && !onceFinished(entry);
	const bot = reportedBots(view).find((value) => value.event_id === eventId);
	return (
		!!bot &&
		bot.hold === null &&
		bot.state !== "waiting" &&
		!isStoppedBot(bot.state)
	);
}

/** A service of another place holds the event: a plan must not put it anywhere else. */
export function holdsSchedule(
	where: ScheduleWhere | undefined,
): where is Extract<
	ScheduleWhere,
	{ fact: "device" | "device_idle" | "device_unconfirmed" }
> {
	return (
		where?.fact === "device" ||
		where?.fact === "device_idle" ||
		where?.fact === "device_unconfirmed"
	);
}

/** The fact names this very service. */
export function isAssignedTo(
	where: ScheduleWhere | undefined,
	deviceId: string,
	serviceId: string,
): boolean {
	return (
		!!where &&
		"deviceId" in where &&
		where.deviceId === deviceId &&
		where.serviceId === serviceId
	);
}

function idleWhy(
	view: Pick<ServiceView, "desired" | "conv">,
): "stopped" | "failed" | null {
	if (view.desired === "stopped" || view.conv === "stopped_by_user")
		return "stopped";
	return view.conv === "crash_looping" || view.conv === "failed_stopped"
		? "failed"
		: null;
}

type Idle = (
	why: ScheduleIdleWhy,
	extra?: { hold?: ScheduleHold; endedAt?: number; botState?: BotStoppedState },
) => ScheduleWhere;

/** A bot's entry: connected or trying to (the device runs it), held, or kept off. */
function botWhere(
	bot: ServiceBot,
	live: boolean,
	assigned: Assigned,
	idle: Idle,
	unconfirmed: () => ScheduleWhere,
): ScheduleWhere {
	if (bot.hold) return idle("held", { hold: bot.hold });
	if (isStoppedBot(bot.state))
		return idle("not_connected", { botState: bot.state });
	// Waiting without a hold: the service has not decided yet.
	if (bot.state === "waiting") return unconfirmed();
	return { fact: "device", bot, live, ...assigned };
}

/** What the service's row says about a claimed event; null when it says nothing. */
function rowWhere(
	view: ServiceView,
	eventId: string,
	assigned: Assigned,
	idle: Idle,
	unconfirmed: () => ScheduleWhere,
): ScheduleWhere | null {
	const live = view.freshness.age === "live";
	const schedule = reportedSchedules(view).find(
		(value) => value.event_id === eventId,
	);
	const finished = schedule && onceFinished(schedule);
	// A finished one-time schedule is a fact about the past: true also for a stopped service.
	if (schedule && finished)
		return { fact: "device", schedule, live, finished, ...assigned };
	const stopped = idleWhy(view);
	if (stopped) return idle(stopped);
	const bot = reportedBots(view).find((value) => value.event_id === eventId);
	if (bot) return botWhere(bot, live, assigned, idle, unconfirmed);
	if (!schedule) return null;
	if (schedule.hold) return idle("held", { hold: schedule.hold });
	return { fact: "device", schedule, live, ...assigned };
}

type ClaimedRow = Extract<AppScheduleRow, { state: "device" }>;

function unconfirmedOf(
	row: ClaimedRow,
	now: number,
	readable: boolean,
): ScheduleWhere {
	return {
		fact: "device_unconfirmed",
		since: row.since,
		seenAt: row.seen_at,
		stale: now - row.seen_at > SCHEDULE_CONFIRM_STALE_S,
		readable,
		...(row.device_id ? { deviceId: row.device_id } : {}),
		...(row.placement_id ? { serviceId: row.placement_id } : {}),
	};
}

/**
 * When the approval of the service a row names ended; undefined while it
 * holds. The hub notices an ended approval only at the schedule's next time;
 * the approval's own facts say it now.
 */
function accessEndedAt(row: ClaimedRow, input: ScheduleWhereInput) {
	const grant = input.placements.placements.find(
		(value) =>
			value.device_id === row.device_id &&
			value.placement_id === row.placement_id,
	)?.grant;
	const ended =
		grant?.status === "revoked" ||
		(grant !== undefined && grant.effective_expires_at <= input.now);
	return ended ? grant?.effective_expires_at : undefined;
}

/** The services of the claim's device when all of them are fresh enough to say whether one runs it. */
function freshViews(input: ScheduleWhereInput, deviceId: string) {
	const services = input.devices.find(
		(device) => device.id === deviceId,
	)?.services;
	if (!Array.isArray(services)) return "unreadable" as const;
	const views = services as readonly ServiceView[];
	return views.every((view) => isFresh(view.freshness)) ? views : "stale";
}

function claimed(row: ClaimedRow, input: ScheduleWhereInput): ScheduleWhere {
	const { device_id: deviceId, placement_id: serviceId } = row;
	const unconfirmed = (readable: boolean) =>
		unconfirmedOf(row, input.now, readable);
	if (!deviceId || !serviceId) return unconfirmed(false);
	const assigned: Assigned = {
		deviceId,
		serviceId,
		since: row.since,
		seenAt: row.seen_at,
	};
	const idle: Idle = (why, extra = {}) => ({
		fact: "device_idle",
		why,
		...extra,
		...assigned,
	});
	const endedAt = accessEndedAt(row, input);
	if (endedAt !== undefined) return idle("access_ended", { endedAt });
	const views = freshViews(input, deviceId);
	// A readable device whose status is too old doesn't say whether it runs; an unreadable one is not known.
	if (!Array.isArray(views)) return unconfirmed(views === "stale");
	const view = views.find((value) => value.serviceId === serviceId);
	if (!view) return idle("removed");
	return (
		rowWhere(view, row.event_id, assigned, idle, () => unconfirmed(true)) ??
		unconfirmed(true)
	);
}

export function scheduleWhere(
	row: AppScheduleRow,
	input: ScheduleWhereInput,
): ScheduleWhere {
	if (row.state === "returning")
		return { fact: "returning", hubResumesAt: row.hub_resumes_at };
	if (row.state === "released")
		return {
			fact: "released",
			since: row.since,
			...(row.device_id ? { deviceId: row.device_id } : {}),
			...(row.placement_id ? { serviceId: row.placement_id } : {}),
			...(row.hub_resumes_at === undefined
				? {}
				: { hubResumesAt: row.hub_resumes_at }),
		};
	return claimed(row, input);
}

/**
 * Every schedule and bot of the app that is not simply run by the hub, by
 * event id. `null`: the hub can't hand schedules to devices, so nothing is
 * known.
 */
export function scheduleWheres(
	placements: AppDevicePlacements,
	input: Omit<ScheduleWhereInput, "placements">,
): Record<string, ScheduleWhere> | null {
	if (!placements.schedules) return null;
	return Object.fromEntries(
		placements.schedules.map((row) => [
			row.event_id,
			scheduleWhere(row, { ...input, placements }),
		]),
	);
}

/** Where a claimed event of an online app runs once the hub's list is known: the hub when it is not listed. */
export function whereOf(
	wheres: Readonly<Record<string, ScheduleWhere>> | null | undefined,
	eventId: string,
): ScheduleWhere | undefined {
	if (!wheres) return undefined;
	return Object.hasOwn(wheres, eventId) ? wheres[eventId] : { fact: "hub" };
}

/**
 * The schedules and bots the hub no longer runs because this service took
 * them, by event id. When the service goes, or its cloud access ends, nothing
 * runs them until they are back on the hub.
 */
export function schedulesClaimedBy(
	rows: readonly AppScheduleRow[] | undefined,
	deviceId: string,
	serviceId: string,
): string[] {
	return (rows ?? [])
		.filter(
			(row) =>
				row.state === "device" &&
				row.device_id === deviceId &&
				row.placement_id === serviceId,
		)
		.map((row) => row.event_id)
		.sort((a, b) => a.localeCompare(b));
}
