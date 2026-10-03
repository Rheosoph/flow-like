import type {
	EventEligibility,
	EventIneligibleCode,
	LatestFlowCase,
} from "../../../../lib/device-management/deployment";
import type { AgentFeature } from "../../../../lib/device-management/model/types";
import type { ScheduleDetail } from "../../../../lib/device-management/schedule";
import type { DevicesT } from "../primitives/area-context";
import { scheduleWords, scheduleZone } from "./schedule-copy";

/* APP §7.3: event types on devices, how an event runs there, and why it can't. */

export type EligibilityFix = "open_events";

export interface EligibilityText {
	/** Step 1, the By event group and popovers. */
	long: string;
	/** The Events "Devices" cell and menus. */
	short: string;
	fix: EligibilityFix | null;
}

export interface EligibilityInput {
	code: EventIneligibleCode;
	eventType: string;
	/**
	 * `flow_error`: the bundle's reason; `refuse` and a device-read schedule: the
	 * device's own words; `route_invalid`: the method or path as written;
	 * `route_reserved`: the path; `bot_invalid`: the setting's key.
	 */
	detail?: string;
	device?: string;
	/** `latest_flow`: which of its cases; an unreadable pin when absent. */
	latestFlow?: LatestFlowCase;
	/** `schedule_invalid`: what this app's own check found. */
	scheduleDetail?: ScheduleDetail;
}

/** The copy input for an event's rule result. */
export function eligibilityInput(
	rule: Pick<
		EventEligibility,
		"code" | "detail" | "latestFlow" | "scheduleDetail"
	> & { code: EventIneligibleCode },
	eventType: string,
	device?: string,
): EligibilityInput {
	return {
		code: rule.code,
		eventType,
		...(rule.detail ? { detail: rule.detail } : {}),
		...(device ? { device } : {}),
		...(rule.latestFlow ? { latestFlow: rule.latestFlow } : {}),
		...(rule.scheduleDetail ? { scheduleDetail: rule.scheduleDetail } : {}),
	};
}

type Pair = Pick<EligibilityText, "long" | "short">;

const TYPE_REASONS = {
	email: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.email.long",
			"Mailboxes are read by the desktop app, not by devices.",
		),
		short: t("devices:eligibility.type.email.short", "Mailboxes can't"),
	}),
	inbound_email: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.inboundEmail.long",
			"Inbound email is handled by the hub.",
		),
		short: t(
			"devices:eligibility.type.inboundEmail.short",
			"Handled by the hub",
		),
	}),
	teams: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.teams.long",
			"Teams bots run on the hub.",
		),
		short: t("devices:eligibility.type.teams.short", "Runs on the hub"),
	}),
	chat_bot: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.chatBot.long",
			"Chat bots run in the desktop app, not on devices.",
		),
		short: t("devices:eligibility.type.chatBot.short", "Chat bots can't"),
	}),
	desktop: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.desktop.long",
			"Deep links and location regions only work in the desktop app.",
		),
		short: t("devices:eligibility.type.desktop.short", "Desktop only"),
	}),
	other: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.other.long",
			"This kind of event can't run on a device.",
		),
		short: t("devices:eligibility.type.other.short", "Can't run on devices"),
	}),
} satisfies Record<string, (t: DevicesT) => Pair>;

type TypeReason = keyof typeof TYPE_REASONS;

const TYPE_GROUPS: Readonly<Record<string, TypeReason>> = {
	email: "email",
	inbound_email: "inbound_email",
	teams: "teams",
	discord: "chat_bot",
	telegram: "chat_bot",
	deeplink: "desktop",
	geolocation: "desktop",
};

const LATEST_FLOW_REASONS: Record<LatestFlowCase, (t: DevicesT) => Pair> = {
	hub: (t) => ({
		long: t(
			"devices:eligibility.latestFlow.hub.long",
			"This hub can't deploy events that follow Latest yet. Update the hub, or pin a flow version in Events.",
		),
		short: t(
			"devices:eligibility.latestFlow.hub.short",
			"Hub can't deploy Latest yet",
		),
	}),
	role: (t) => ({
		long: t(
			"devices:eligibility.latestFlow.role.long",
			"This flow has edits that aren't published as a version, and your role can't create one. Ask someone who can edit the app to create a version.",
		),
		short: t(
			"devices:eligibility.latestFlow.role.short",
			"Flow edits need a version",
		),
	}),
	target: (t) => ({
		long: t(
			"devices:eligibility.latestFlow.target.long",
			"This event points at a Page or a start node that is no longer in its flow. Open it in Events and pick it again.",
		),
		short: t(
			"devices:eligibility.latestFlow.target.short",
			"Page or start node is gone",
		),
	}),
	copy: (t) => ({
		long: t(
			"devices:eligibility.latestFlow.copy.long",
			"This copy has no published version of the event's flow. Export it again from the desktop app.",
		),
		short: t(
			"devices:eligibility.latestFlow.copy.short",
			"Copy has no flow version",
		),
	}),
	other: (t) => ({
		long: t(
			"devices:eligibility.latestFlow.other.long",
			"Its flow version can't be read. Pick a flow version in Events.",
		),
		short: t(
			"devices:eligibility.latestFlow.other.short",
			"Flow version can't be read",
		),
	}),
};

const LATEST_FLOW_FIXES: Record<LatestFlowCase, EligibilityFix | null> = {
	hub: "open_events",
	role: null,
	target: "open_events",
	copy: null,
	other: "open_events",
};

const SCHEDULE_DETAILS: {
	[C in ScheduleDetail["code"]]: (
		t: DevicesT,
		detail: Extract<ScheduleDetail, { code: C }>,
	) => string;
} = {
	both: (t) =>
		t(
			"devices:eligibility.schedule.detail.both",
			"it has a repeating schedule and a one-time date",
		),
	length: (t) =>
		t(
			"devices:eligibility.schedule.detail.length",
			"the expression is longer than 128 characters",
		),
	fields: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.fields",
			"the expression has {{count}} fields, and a device reads 5 or 6",
			{ count: detail.count },
		),
	syntax: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.syntax",
			"“{{field}}” is a form devices don't read",
			{ field: detail.field },
		),
	parser: (t) =>
		t(
			"devices:eligibility.schedule.detail.parser",
			"the expression isn't valid or never matches a date",
		),
	zone: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.zone",
			"the time zone {{zone}} isn't known",
			{ zone: detail.zone },
		),
	date: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.date",
			"the date {{date}} isn't a real day written as YYYY-MM-DD",
			{ date: detail.date },
		),
	time: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.time",
			"the time {{time}} isn't a time from 00:00 to 23:59",
			{ time: detail.time },
		),
	gap: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.gap",
			"{{date}} {{time}} doesn't exist in {{zone}}: the clocks skip it",
			{ date: detail.date, time: detail.time, zone: detail.zone },
		),
	range: (t, detail) =>
		t(
			"devices:eligibility.schedule.detail.range",
			"the date {{date}} is before 2000 or after 2100",
			{ date: detail.date },
		),
};

function scheduleDetailCopy(t: DevicesT, input: EligibilityInput): string {
	if (input.detail) return input.detail;
	const detail = input.scheduleDetail ?? { code: "parser" };
	return (
		SCHEDULE_DETAILS[detail.code] as (
			t: DevicesT,
			detail: ScheduleDetail,
		) => string
	)(t, detail);
}

/** What an older hub can't hand to a device, by the event's type. */
function hubTypeLong(t: DevicesT, eventType: string): string {
	if (eventType === "telegram" || eventType === "discord")
		return t(
			"devices:eligibility.hubType.bot.long",
			"This hub can't hand bots to devices yet. Update the hub.",
		);
	if (eventType === "quick_action" || eventType === "generic_form")
		return t(
			"devices:eligibility.hubType.onDemand.long",
			"This hub can't hand forms and quick actions to devices yet. Update the hub.",
		);
	return t(
		"devices:eligibility.hubType.endpoint.long",
		"This hub can't hand Endpoints to devices yet. Update the hub.",
	);
}

const REASONS = {
	paused: (t) => ({
		long: t(
			"devices:eligibility.paused.long",
			"Paused. Activate it in Events first.",
		),
		short: t("devices:eligibility.paused.short", "Paused: activate it first"),
	}),
	latest_flow: (t, input) =>
		LATEST_FLOW_REASONS[input.latestFlow ?? "other"](t),
	canary: (t) => ({
		long: t(
			"devices:eligibility.canary.long",
			"Splits traffic with a canary. A device can't split traffic; end the canary first.",
		),
		short: t(
			"devices:eligibility.canary.short",
			"Splits traffic with a canary",
		),
	}),
	variants: (t) => ({
		long: t(
			"devices:eligibility.variants.long",
			"Has traffic variants. A device can't split traffic.",
		),
		short: t("devices:eligibility.variants.short", "Has traffic variants"),
	}),
	type: (t, input) =>
		TYPE_REASONS[
			Object.hasOwn(TYPE_GROUPS, input.eventType)
				? TYPE_GROUPS[input.eventType]
				: "other"
		](t),
	route_missing: (t) => ({
		long: t(
			"devices:eligibility.route.missing.long",
			"It has no path. Set one in Events first.",
		),
		short: t("devices:eligibility.route.missing.short", "No path set"),
	}),
	route_invalid: (t, input) => ({
		long: t(
			"devices:eligibility.route.invalid.long",
			"A device can't serve its method or path: {{detail}}",
			{ detail: input.detail ?? "" },
		),
		short: t("devices:eligibility.route.invalid.short", "Path can't be served"),
	}),
	route_reserved: (t, input) => ({
		long: t(
			"devices:eligibility.route.reserved.long",
			"Its path {{path}} is used by the service itself. Choose another path in Events.",
			{ path: input.detail ?? "" },
		),
		short: t("devices:eligibility.route.reserved.short", "Path is taken"),
	}),
	schedule_missing: (t) => ({
		long: t(
			"devices:eligibility.schedule.missing.long",
			"No schedule set. Set one in Events first.",
		),
		short: t("devices:eligibility.schedule.missing.short", "No schedule set"),
	}),
	schedule_once: (t) => ({
		long: t(
			"devices:eligibility.schedule.once.long",
			"This device's agent can't run one-time schedules yet. Update the device agent.",
		),
		short: t("devices:eligibility.schedule.once.short", "Agent too old"),
	}),
	schedule_invalid: (t, input) => ({
		long: t(
			"devices:eligibility.schedule.invalid.long",
			"Its schedule can't be read on a device: {{detail}}",
			{ detail: scheduleDetailCopy(t, input) },
		),
		short: t(
			"devices:eligibility.schedule.invalid.short",
			"Schedule can't be read",
		),
	}),
	schedule_too_often: (t) => ({
		long: t(
			"devices:eligibility.schedule.tooOften.long",
			"Runs more often than once a minute. Devices run a schedule at most once a minute.",
		),
		short: t(
			"devices:eligibility.schedule.tooOften.short",
			"More than once a minute",
		),
	}),
	bot_invalid: (t, input) => ({
		long: t(
			"devices:eligibility.bot.invalid.long",
			"Its bot settings can't be read on a device: {{detail}}",
			{ detail: input.detail ?? "" },
		),
		short: t(
			"devices:eligibility.bot.invalid.short",
			"Bot settings can't be read",
		),
	}),
	hub_schedules: (t) => ({
		long: t(
			"devices:eligibility.hubSchedules.long",
			"This hub can't run schedules on devices yet. Update the hub.",
		),
		short: t(
			"devices:eligibility.hubSchedules.short",
			"Hub can't run schedules on devices yet",
		),
	}),
	hub_type: (t, input) => ({
		long: hubTypeLong(t, input.eventType),
		short: t("devices:eligibility.hubType.short", "Hub can't yet"),
	}),
	flow_error: (t, input) => ({
		long: t(
			"devices:eligibility.flowError.long",
			"Its flow version can't be prepared for devices: {{detail}}",
			{ detail: input.detail ?? "" },
		),
		short: t("devices:eligibility.flowError.short", "Flow can't be prepared"),
	}),
	refuse: (t, input) => ({
		long: t(
			"devices:eligibility.refuse.long",
			"{{device}} can't run it: {{reason}}",
			{
				device:
					input.device ?? t("devices:eligibility.thisDevice", "This device"),
				reason: input.detail ?? "",
			},
		),
		short: t("devices:eligibility.refuse.short", "Can't run here"),
	}),
} satisfies Record<
	EventIneligibleCode,
	(t: DevicesT, input: EligibilityInput) => Pair
>;

const FIXES: Record<
	Exclude<EventIneligibleCode, "latest_flow">,
	EligibilityFix | null
> = {
	paused: "open_events",
	canary: "open_events",
	variants: "open_events",
	type: null,
	route_missing: "open_events",
	route_invalid: "open_events",
	route_reserved: "open_events",
	schedule_missing: "open_events",
	schedule_once: null,
	schedule_invalid: "open_events",
	schedule_too_often: "open_events",
	bot_invalid: "open_events",
	hub_schedules: null,
	hub_type: null,
	flow_error: null,
	refuse: null,
};

/** Why an event can't run on devices, in the long and the short form, with its fix link. */
export function eligibilityCopy(
	t: DevicesT,
	input: EligibilityInput,
): EligibilityText {
	return {
		...REASONS[input.code](t, input),
		fix:
			input.code === "latest_flow"
				? LATEST_FLOW_FIXES[input.latestFlow ?? "other"]
				: FIXES[input.code],
	};
}

export function eligibilityFixLabel(t: DevicesT, _fix: EligibilityFix): string {
	return t("devices:eligibility.fix.openEvents", "Open Events");
}

type FeatureCopy = (
	t: DevicesT,
	device: string,
) => { long: string; fix: string };

/** One sentence and one fix per agent flag an event can need. */
const AGENT_FEATURE_COPY: Partial<Record<AgentFeature, FeatureCopy>> = {
	api_events: (t, device) => ({
		long: t(
			"devices:eligibility.agent.apiEvents.long",
			"{{device}}'s agent is too old to serve Endpoints.",
			{ device },
		),
		fix: t(
			"devices:eligibility.agent.apiEvents.fix",
			"Update the device agent to serve Endpoints",
		),
	}),
	scheduled_once: (t, device) => ({
		long: t(
			"devices:eligibility.agent.scheduledOnce.long",
			"{{device}}'s agent is too old to run one-time schedules.",
			{ device },
		),
		fix: t(
			"devices:eligibility.agent.scheduledOnce.fix",
			"Update the device agent to run one-time schedules",
		),
	}),
	on_demand_events: (t, device) => ({
		long: t(
			"devices:eligibility.agent.onDemandEvents.long",
			"{{device}}'s agent is too old to run forms and quick actions.",
			{ device },
		),
		fix: t(
			"devices:eligibility.agent.onDemandEvents.fix",
			"Update the device agent to run forms and quick actions",
		),
	}),
	telegram_bots: (t, device) => ({
		long: t(
			"devices:eligibility.agent.telegramBots.long",
			"{{device}}'s agent is too old to run Telegram bots.",
			{ device },
		),
		fix: t(
			"devices:eligibility.agent.telegramBots.fix",
			"Update the device agent to run Telegram bots",
		),
	}),
	discord_bots: (t, device) => ({
		long: t(
			"devices:eligibility.agent.discordBots.long",
			"{{device}}'s agent is too old to run Discord bots.",
			{ device },
		),
		fix: t(
			"devices:eligibility.agent.discordBots.fix",
			"Update the device agent to run Discord bots",
		),
	}),
};

/**
 * One device's agent lacks the flag an event needs: a reason per device,
 * never an app-level code. Without a flag, the schedules sentence of round one.
 */
export function agentTooOldCopy(
	t: DevicesT,
	device: string,
	/** An `AgentFeature`; a plan param arrives as text. */
	feature?: string,
): Pair & { fix: string } {
	const short = t("devices:eligibility.agent.short", "Agent too old");
	const copy =
		feature && Object.hasOwn(AGENT_FEATURE_COPY, feature)
			? AGENT_FEATURE_COPY[feature as AgentFeature]
			: undefined;
	if (copy) return { ...copy(t, device), short };
	return {
		long: t(
			"devices:eligibility.agent.long",
			"{{device}}'s agent is too old to run schedules.",
			{ device },
		),
		short,
		fix: t(
			"devices:eligibility.agent.fix",
			"Update the device agent to run schedules",
		),
	};
}

export interface CantHereCell {
	why?: "refuse" | "agent" | "runs_elsewhere";
	reason?: string;
	/** `agent`: the first flag the device's agent lacks. */
	feature?: AgentFeature;
	/** `runs_elsewhere`: the event is a bot, not a schedule. */
	bot?: boolean;
}

/** Why one device can't take an event that can run elsewhere: its agent, another place that holds it, or its own words. */
export function cantHereCopy(
	t: DevicesT,
	cell: CantHereCell,
	device: string,
): string {
	if (cell.why === "agent")
		return agentTooOldCopy(t, device, cell.feature).long;
	if (cell.why === "runs_elsewhere")
		return cell.bot
			? t(
					"devices:eligibility.runsElsewhereBot",
					"Another service runs this bot. A bot runs in one place.",
				)
			: t(
					"devices:eligibility.runsElsewhere",
					"Another service runs this schedule. A schedule runs in one place.",
				);
	return cell.reason ?? "";
}

type RunsRule = Pick<EventEligibility, "hosted" | "readiness"> &
	Partial<
		Pick<EventEligibility, "kind" | "schedule" | "once" | "bot" | "route">
	>;

/** The line under an event that can run: an Endpoint's method and path, when a schedule runs and in which zone, else how a device runs it. */
export function eventRunsCopy(t: DevicesT, rule: RunsRule): string {
	if (rule.route)
		return t(
			"devices:eligibility.runs.route",
			"{{method}} {{path}} · served by the device",
			{ method: rule.route.method, path: rule.route.path },
		);
	if (rule.kind === "scheduled" && rule.once)
		return t(
			"devices:eligibility.runs.onceAt",
			"Once on {{date}} at {{time}} · {{zone}}",
			{
				date: rule.once.date,
				time: rule.once.time,
				zone: scheduleZone(t, rule.once),
			},
		);
	return rule.kind === "scheduled" && rule.schedule
		? t("devices:eligibility.runs.schedule", "{{words}} · {{zone}}", {
				words: scheduleWords(t, rule.schedule.expression),
				zone: scheduleZone(t, rule.schedule),
			})
		: howItRunsCopy(t, rule);
}

/** How a device runs an event of a kind that has words of its own; null for the others. */
function kindRunsCopy(t: DevicesT, rule: RunsRule) {
	switch (rule.kind) {
		case "scheduled":
			return rule.once
				? t("devices:eligibility.runs.once", "Runs once · the device starts it")
				: t(
						"devices:eligibility.runs.scheduled",
						"Runs on a schedule · the device starts it",
					);
		case "on_demand":
			return t(
				"devices:eligibility.runs.onDemand",
				"Started by a person · from Devices or the service page",
			);
		case "bot":
			return rule.bot?.provider === "discord"
				? t(
						"devices:eligibility.runs.bot.discord",
						"Runs on its own · stays connected to Discord",
					)
				: t(
						"devices:eligibility.runs.bot.telegram",
						"Runs on its own · stays connected to Telegram",
					);
		default:
			return null;
	}
}

/** How an eligible event runs on a device (step 1, By event, popovers). */
export function howItRunsCopy(t: DevicesT, rule: RunsRule): string {
	const own = kindRunsCopy(t, rule);
	if (own) return own;
	if (rule.hosted)
		return t(
			"devices:eligibility.runs.hosted",
			"Served by the device · checks its web server",
		);
	if (rule.readiness === "listener")
		return t(
			"devices:eligibility.runs.ownServer",
			"Runs its own server · checks it answers",
		);
	return rule.readiness === "explicit"
		? t(
				"devices:eligibility.runs.explicit",
				"Runs on its own · waits for the flow to report ready",
			)
		: t(
				"devices:eligibility.runs.unsupported",
				"Runs on its own · can't be checked, so no safe updates",
			);
}

const endpoint = (t: DevicesT) =>
	t("devices:eligibility.eventType.api", "Endpoint");

const EVENT_TYPES = {
	simple_chat: (t: DevicesT) => t("devices:eligibility.eventType.chat", "Chat"),
	page: (t: DevicesT) => t("devices:eligibility.eventType.page", "Page"),
	http: endpoint,
	api: endpoint,
	rest: (t: DevicesT) => t("devices:eligibility.eventType.rest", "REST"),
	mcp: (t: DevicesT) => t("devices:eligibility.eventType.mcp", "MCP"),
	daemon: (t: DevicesT) =>
		t("devices:eligibility.eventType.daemon", "Background"),
	cron: (t: DevicesT) => t("devices:eligibility.eventType.cron", "Schedule"),
	email: (t: DevicesT) => t("devices:eligibility.eventType.email", "Mailbox"),
	inbound_email: (t: DevicesT) =>
		t("devices:eligibility.eventType.inboundEmail", "Inbound email"),
	teams: (t: DevicesT) => t("devices:eligibility.eventType.teams", "Teams bot"),
	discord: (t: DevicesT) =>
		t("devices:eligibility.eventType.discord", "Discord bot"),
	telegram: (t: DevicesT) =>
		t("devices:eligibility.eventType.telegram", "Telegram bot"),
	generic_form: (t: DevicesT) =>
		t("devices:eligibility.eventType.form", "Form"),
	quick_action: (t: DevicesT) =>
		t("devices:eligibility.eventType.quickAction", "Quick action"),
	deeplink: (t: DevicesT) =>
		t("devices:eligibility.eventType.deeplink", "Deep link"),
	geolocation: (t: DevicesT) =>
		t("devices:eligibility.eventType.geolocation", "Location region"),
} satisfies Record<string, (t: DevicesT) => string>;

/** Device label of an event type (APP §7.3); an event with a page reads "Page". */
export function eventTypeLabel(
	t: DevicesT,
	eventType: string,
	hasPage = false,
): string {
	const key = hasPage ? "page" : eventType;
	return Object.hasOwn(EVENT_TYPES, key)
		? EVENT_TYPES[key as keyof typeof EVENT_TYPES](t)
		: t("devices:eligibility.eventType.other", "Other");
}
