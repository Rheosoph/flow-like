import {
	botTokenEventId,
	botTokenKey,
	isBotTokenKey,
} from "../../../../lib/device-management/bot-config";
import {
	type DeploymentEvent,
	type EventEligibility,
	eventEligibility,
	missingFeature,
} from "../../../../lib/device-management/deployment";
import { routeConflicts } from "../../../../lib/device-management/event-route";
import type {
	AppEventInput,
	AppInput,
} from "../../../../lib/device-management/model/app-plan";
import type {
	AgentFeature,
	AgentFeatures,
	ScheduleHold,
	ScheduleOutcome,
	ServiceAction,
	ServiceBot,
	ServiceSchedule,
} from "../../../../lib/device-management/model/types";
import { nextRuns } from "../../../../lib/device-management/schedule";
import {
	type FakeEventForm,
	type FakeRunCounters,
	fileFields,
	formKind,
} from "./fake-runs";

/*
 * What a fake agent makes of a service's events (design R2 §1.1, §1.5, §1.7):
 * the discovery rows of an app copy, the refusals that fail a start, and the
 * row facts of a running service (schedules of both kinds, bots, actions and
 * forms) within the agent's bounds. The rule is the client's own
 * (`eventEligibility`, `missingFeature`), applied the way an agent of the
 * fake's flags applies it.
 */

type Triple = [number, number, number];

/** The flag an agent needs to know a type at all; without it the type is unknown to it. */
const TYPE_FLAG: Record<string, AgentFeature> = {
	api: "api_events",
	quick_action: "on_demand_events",
	generic_form: "on_demand_events",
	telegram: "telegram_bots",
	discord: "discord_bots",
};
const ROUTE_CODES: ReadonlySet<string> = new Set([
	"route_missing",
	"route_invalid",
	"route_reserved",
]);
const SCHEDULE_CODES: ReadonlySet<string> = new Set([
	"schedule_missing",
	"schedule_once",
	"schedule_invalid",
	"schedule_too_often",
]);

/** An event of a type this agent was built without: unknown, as a round-one agent answers it. */
const unknownType = (
	event: AppEventInput,
	features: AgentFeatures,
): boolean => {
	const flag = TYPE_FLAG[event.event_type];
	return !event.default_page_id && !!flag && features[flag] !== 1;
};

/** The device's verdict on one event: the client's rule as an agent with these flags reads it. */
export function agentRule(
	event: AppEventInput,
	features: AgentFeatures,
): EventEligibility & { unknown: boolean } {
	const rule = eventEligibility(event);
	if (unknownType(event, features))
		return {
			...rule,
			unknown: true,
			eligible: false,
			code: "type",
			kind: null,
			hosted: false,
			readiness: "unsupported",
			rolloutSupported: false,
		};
	// An agent before Endpoints reads routes only at start: discovery accepts what it can't serve.
	if (features.api_events !== 1 && rule.code && ROUTE_CODES.has(rule.code))
		return {
			...rule,
			unknown: false,
			eligible: true,
			code: null,
			detail: undefined,
			rolloutSupported: true,
		};
	if (rule.once && features.scheduled_once !== 1)
		return {
			...rule,
			unknown: false,
			eligible: false,
			code: rule.code ?? "schedule_once",
			once: undefined,
			rolloutSupported: false,
		};
	return { ...rule, unknown: false };
}

const OLD_AGENT_UNSUPPORTED = {
	eligible: false,
	readiness_kind: "unsupported",
	rollout_supported: false,
} as const;

function readinessError(id: string, code: string | null): string | null {
	if (!code) return null;
	if (SCHEDULE_CODES.has(code))
		return `Event ${id} has a schedule this device can't run (${code}).`;
	if (ROUTE_CODES.has(code))
		return `Event ${id} has a route this device can't serve (${code}).`;
	return code === "bot_invalid"
		? `Event ${id} has bot settings this device can't read (${code}).`
		: null;
}

/**
 * The rows of `artifact · describe` for an app. A Latest event carries the pin
 * the export gave it (`pinOf`), or none when no version equals its flow: the
 * device then refuses it. An agent without `scheduled_events` sends none of
 * the newer fields and refuses every schedule; one built without a part of
 * round two answers that type as unknown.
 */
export function discoveryRows(
	app: AppInput,
	features: AgentFeatures,
	pinOf: (event: AppEventInput) => Triple | null,
): DeploymentEvent[] {
	const facts = features.scheduled_events === 1;
	return app.events
		.map((event): DeploymentEvent => {
			const latest = eventEligibility(event).followsLatest;
			const boardVersion = latest ? pinOf(event) : null;
			const pinned =
				latest && boardVersion
					? { ...event, board_version: boardVersion }
					: event;
			const rule = agentRule(pinned, features);
			const unpinned = latest && !boardVersion;
			const base = {
				id: event.id,
				name: event.name,
				event_type: event.event_type,
				event_version: rule.eventVersion,
				board_version: unpinned ? null : rule.boardVersion,
				hosted: rule.hosted,
				readiness_kind: rule.readiness,
				rollout_supported: rule.rolloutSupported,
				eligible: rule.eligible && !unpinned,
			};
			if (!facts)
				return rule.kind === "scheduled" || rule.unknown
					? { ...base, ...OLD_AGENT_UNSUPPORTED }
					: base;
			const code =
				rule.code &&
				(SCHEDULE_CODES.has(rule.code) ||
					ROUTE_CODES.has(rule.code) ||
					rule.code === "bot_invalid")
					? rule.code
					: null;
			return {
				...base,
				readiness_error: readinessError(event.id, code),
				...(rule.kind ? { kind: rule.kind } : {}),
				...(rule.schedule
					? {
							schedule: {
								expression: rule.schedule.expression,
								timezone: rule.schedule.timezone,
							},
						}
					: {}),
				...(rule.once
					? {
							once: {
								date: rule.once.date,
								time: rule.once.time,
								at: rule.once.at,
								timezone: rule.once.timezone,
							},
						}
					: {}),
				...(rule.route && features.api_events === 1
					? { route: rule.route }
					: {}),
				ineligible_code: code as DeploymentEvent["ineligible_code"],
			};
		})
		.sort((a, b) => (a.id < b.id ? -1 : 1));
}

/* A service's events as its process reads them. */

export interface ServiceEvent {
	event: AppEventInput;
	rule: EventEligibility;
}

/** The service's events in its config order, with the rule each one passed. */
export function serviceEvents(
	events: readonly AppEventInput[],
	eventIds: readonly string[],
): ServiceEvent[] {
	return eventIds.flatMap((id) => {
		const event = events.find((candidate) => candidate.id === id);
		return event ? [{ event, rule: eventEligibility(event) }] : [];
	});
}

/** Self-firing events that a claim keeps in one place: schedules of both kinds and bots. */
export const isClaimed = ({ rule }: ServiceEvent) =>
	rule.kind === "scheduled" || rule.kind === "bot";

const SELF_FIRING_LIMIT = 64;

function flagRefusal(event: AppEventInput, missing: AgentFeature): string {
	if (missing === "scheduled_once")
		return `Event ${event.id} has a one-time schedule this device can't run.`;
	if (missing === "api_events" && event.event_type === "http")
		return `Event ${event.id} has a route this device can't read.`;
	return `Event ${event.id} needs an unsupported ${event.event_type} sink.`;
}

function ruleRefusal({ event, rule }: ServiceEvent): string | null {
	if (!rule.code) return null;
	if (ROUTE_CODES.has(rule.code))
		return `Event ${event.id} has a route this device can't serve: ${rule.detail ?? rule.code}.`;
	if (SCHEDULE_CODES.has(rule.code))
		return `Event ${event.id} has a schedule this device can't run (${rule.code}).`;
	return rule.code === "bot_invalid"
		? `Event ${event.id} has bot settings this device can't read: ${rule.detail ?? "config"}.`
		: null;
}

/**
 * Why a process of the service fails to start (validation in `sa/runtime.rs`),
 * or null. `secretOverrides` is null when the fake does not know the service's
 * settings (a seeded service): then no token key is checked.
 */
export function startRefusal(
	events: readonly ServiceEvent[],
	features: AgentFeatures,
	secretOverrides: Readonly<Record<string, unknown>> | null,
	hasWebEndpoint: boolean,
): string | null {
	for (const entry of events) {
		const missing = missingFeature(entry.event, features);
		if (missing && missing !== "unknown")
			return flagRefusal(entry.event, missing);
	}
	for (const entry of events) {
		const refusal = ruleRefusal(entry);
		if (refusal) return refusal;
	}
	if (
		routeConflicts(
			events.map(({ event, rule }) => ({ ...event, route: rule.route })),
			hasWebEndpoint,
		).length
	)
		return "Two events claim the same method and service path";
	if (events.filter(isClaimed).length > SELF_FIRING_LIMIT)
		return "A service runs at most 64 schedules and bots";
	if (!secretOverrides) return null;
	const bots = new Set(
		events
			.filter(({ rule }) => rule.kind === "bot")
			.map(({ event }) => event.id),
	);
	for (const key of Object.keys(secretOverrides))
		if (isBotTokenKey(key) && !bots.has(botTokenEventId(key) ?? ""))
			return `placement variable ${key} is absent from the selected pinned boards`;
	for (const id of bots)
		if (!Object.hasOwn(secretOverrides, botTokenKey(id)))
			return `Event ${id} has no bot token.`;
	return null;
}

/* One-time schedules: the record per event and instant (§3.2). */

export const ONCE_LATE_S = 900;

export interface OnceRecord {
	eventId: string;
	onceAt: number;
	timezone: string;
	eventVersion: readonly number[];
	state: "pending" | "started" | "ran" | "missed" | "passed";
	armedAt: number | null;
	lastAt: number | null;
	lastOutcome: ScheduleOutcome | null;
}

const FINAL: ReadonlySet<OnceRecord["state"]> = new Set([
	"ran",
	"missed",
	"passed",
]);
export const onceFinished = (record: OnceRecord) => FINAL.has(record.state);

/**
 * Arms or ends one record as the scheduler does when it arms (rules 2–5).
 * `floor` is the claim's `since` of an online service.
 */
export function armOnce(
	record: OnceRecord,
	hold: ScheduleHold | null,
	now: number,
	floor: number | null,
	outcome: ScheduleOutcome,
): void {
	if (onceFinished(record)) return;
	if (record.state === "started") {
		Object.assign(record, {
			state: "ran",
			lastAt: now,
			lastOutcome: "cancelled",
		});
		return;
	}
	if (hold) {
		if (record.onceAt <= now) record.state = "passed";
		return;
	}
	if (record.armedAt === null) {
		if (record.onceAt > now && (floor === null || record.onceAt > floor))
			record.armedAt = now;
		else record.state = "passed";
		return;
	}
	runIfDue(record, now, floor, outcome);
}

/** A running process reaches the instant (rule 5 and 6): runs it up to 15 minutes late, else missed. */
export function runIfDue(
	record: OnceRecord,
	now: number,
	floor: number | null,
	outcome: ScheduleOutcome,
): void {
	if (onceFinished(record) || record.armedAt === null || now < record.onceAt)
		return;
	if (
		now <= record.onceAt + ONCE_LATE_S &&
		(floor === null || floor < record.onceAt)
	)
		Object.assign(record, { state: "ran", lastAt: now, lastOutcome: outcome });
	else record.state = "missed";
}

/** The live entry of a one-time schedule; `null` hold once it finished. */
export function liveOnce(
	record: OnceRecord,
	hold: ScheduleHold | null,
): ServiceSchedule {
	const armed = record.state === "pending" && record.armedAt !== null;
	return {
		event_id: record.eventId,
		once_at: record.onceAt,
		timezone: record.timezone,
		once_state: record.state,
		hold: onceFinished(record) ? null : hold,
		next_at: armed && !hold ? record.onceAt : null,
		running: false,
		last_at: record.lastAt,
		last_outcome: record.lastOutcome,
	};
}

/** One repeating schedule as a running service reports it live, armed or held. */
export function liveSchedule(
	schedule: { eventId: string; expression: string; timezone: string },
	hold: ScheduleHold | null,
	now: number,
): ServiceSchedule {
	const [next] = hold ? [] : nextRuns(schedule, now * 1_000, 1);
	return {
		event_id: schedule.eventId,
		expression: schedule.expression,
		timezone: schedule.timezone,
		hold,
		next_at: next === undefined ? null : Math.floor(next / 1_000),
		running: false,
		last_at: null,
		last_outcome: null,
		runs: 0,
		failed: 0,
		skipped: 0,
		last_skip: null,
		clock_behind: false,
	};
}

/* Bots and actions. */

const BOT_NAME = /^[A-Za-z0-9_.\- ]{1,64}$/;

/** A bot as a running service reports it: held, else connected unless the test said otherwise. */
export function liveBot(
	event: AppEventInput,
	hold: ScheduleHold | null,
	since: number,
	facts: Partial<ServiceBot> = {},
): ServiceBot {
	const state = hold ? "waiting" : (facts.state ?? "connected");
	const connected = state === "connected" || state === "reconnecting";
	return {
		event_id: event.id,
		provider: event.event_type === "discord" ? "discord" : "telegram",
		bot_name: BOT_NAME.test(event.name) ? event.name : null,
		connected_at: connected ? since : null,
		last_message_at: null,
		last_outcome: null,
		running: 0,
		runs: 0,
		runs_today: 0,
		failed: 0,
		dropped: 0,
		...facts,
		state,
		hold,
	};
}

export function liveAction(
	event: AppEventInput,
	form: FakeEventForm,
	counters: FakeRunCounters,
): ServiceAction {
	return {
		event_id: event.id,
		kind: formKind(form),
		fields: form.fields.length,
		file_fields: fileFields(form.fields),
		running: counters.running,
		last_at: counters.last_at,
		last_outcome: counters.last_outcome,
		runs: counters.runs,
		failed: counters.failed,
	};
}

/* The row lists and their bounds (§1.7). */

const LIST_BUDGET = 4_096;
const MAX_SCHEDULES = 16;
const MAX_BOTS = 8;
const MAX_ACTIONS = 16;

export interface RowLists {
	schedules?: ServiceSchedule[];
	schedules_truncated?: boolean;
	bots?: ServiceBot[];
	bots_truncated?: boolean;
	actions?: ServiceAction[];
	actions_truncated?: boolean;
}

const sizeOf = (lists: unknown[][]) =>
	new TextEncoder().encode(JSON.stringify(lists)).length;

/**
 * The three lists of one row within their counts and their shared 4 KiB:
 * entries go from the end of `actions`, then `schedules`, then `bots`, and a
 * list that lost entries says so. An absent list stays absent.
 */
export function boundedLists(lists: {
	schedules?: ServiceSchedule[];
	bots?: ServiceBot[];
	actions?: ServiceAction[];
}): RowLists {
	const schedules = lists.schedules?.slice(0, MAX_SCHEDULES);
	const bots = lists.bots?.slice(0, MAX_BOTS);
	const actions = lists.actions?.slice(0, MAX_ACTIONS);
	const kept = [actions, schedules, bots];
	const all = () => kept.map((list) => list ?? []);
	for (const list of kept)
		while (list?.length && sizeOf(all()) > LIST_BUDGET) list.pop();
	return {
		...(schedules
			? {
					schedules,
					schedules_truncated:
						schedules.length < (lists.schedules?.length ?? 0),
				}
			: {}),
		...(bots
			? { bots, bots_truncated: bots.length < (lists.bots?.length ?? 0) }
			: {}),
		...(actions
			? {
					actions,
					actions_truncated: actions.length < (lists.actions?.length ?? 0),
				}
			: {}),
	};
}

/** What a snapshot says about a schedule, a bot and an action: nothing that changes by itself. */
export function stableSchedule(entry: ServiceSchedule): ServiceSchedule {
	const {
		next_at: _next,
		running: _running,
		last_at: _last,
		runs: _runs,
		failed: _failed,
		skipped: _skipped,
		last_skip: _skip,
		clock_behind: _behind,
		...stable
	} = entry;
	return stable;
}

const LIVE_ONLY_STATES: ReadonlySet<string> = new Set([
	"connecting",
	"connected",
	"reconnecting",
]);

export function stableBot(entry: ServiceBot): ServiceBot {
	return {
		event_id: entry.event_id,
		provider: entry.provider,
		hold: entry.hold,
		state: LIVE_ONLY_STATES.has(entry.state) ? "ok" : entry.state,
	};
}

export function stableAction(entry: ServiceAction): ServiceAction {
	return {
		event_id: entry.event_id,
		kind: entry.kind,
		fields: entry.fields,
		file_fields: entry.file_fields,
	};
}
