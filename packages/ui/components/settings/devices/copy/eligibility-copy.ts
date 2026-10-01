import type {
	EventEligibility,
	EventIneligibleCode,
} from "../../../../lib/device-management/deployment";
import type { DevicesT } from "../primitives/area-context";

/* APP §7.3: event types on devices, how an event runs there, and why it can't. */

export type EligibilityFix = "open_events" | "pin_flow";

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
	/** `flow_error`: the bundle's reason; `refuse`: the device's own words. */
	detail?: string;
	device?: string;
}

type Pair = Pick<EligibilityText, "long" | "short">;

const TYPE_REASONS = {
	cron: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.cron.long",
			"Schedules can't run on a device yet.",
		),
		short: t("devices:eligibility.type.cron.short", "Schedules can't"),
	}),
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
	in_app: (t: DevicesT): Pair => ({
		long: t(
			"devices:eligibility.type.inApp.long",
			"Forms and quick actions open inside Flow-Like. Give it a page to serve it from a device.",
		),
		short: t("devices:eligibility.type.inApp.short", "Opens inside Flow-Like"),
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
	cron: "cron",
	email: "email",
	inbound_email: "inbound_email",
	teams: "teams",
	discord: "chat_bot",
	telegram: "chat_bot",
	generic_form: "in_app",
	quick_action: "in_app",
	deeplink: "desktop",
	geolocation: "desktop",
};

const REASONS = {
	paused: (t) => ({
		long: t(
			"devices:eligibility.paused.long",
			"Paused. Activate it in Events first.",
		),
		short: t("devices:eligibility.paused.short", "Paused: activate it first"),
	}),
	latest_flow: (t) => ({
		long: t(
			"devices:eligibility.latestFlow.long",
			"Follows the latest flow edits. Pin a flow version in Events first.",
		),
		short: t("devices:eligibility.latestFlow.short", "Follows the latest flow"),
	}),
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
	api_type: (t) => ({
		long: t(
			"devices:eligibility.apiType.long",
			"Endpoint events can't run on a device yet. Use a REST event, or give it a page.",
		),
		short: t("devices:eligibility.apiType.short", "Endpoint events can't yet"),
	}),
	type: (t, input) =>
		TYPE_REASONS[
			Object.hasOwn(TYPE_GROUPS, input.eventType)
				? TYPE_GROUPS[input.eventType]
				: "other"
		](t),
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

const FIXES: Record<EventIneligibleCode, EligibilityFix | null> = {
	paused: "open_events",
	latest_flow: "pin_flow",
	canary: "open_events",
	variants: "open_events",
	api_type: null,
	type: null,
	flow_error: null,
	refuse: null,
};

/** Why an event can't run on devices, in the long and the short form, with its fix link. */
export function eligibilityCopy(
	t: DevicesT,
	input: EligibilityInput,
): EligibilityText {
	return { ...REASONS[input.code](t, input), fix: FIXES[input.code] };
}

export function eligibilityFixLabel(t: DevicesT, fix: EligibilityFix): string {
	return fix === "pin_flow"
		? t("devices:eligibility.fix.pinFlow", "Pin a flow version in Events")
		: t("devices:eligibility.fix.openEvents", "Open Events");
}

/** How an eligible event runs on a device (step 1, By event, popovers). */
export function howItRunsCopy(
	t: DevicesT,
	rule: Pick<EventEligibility, "hosted" | "readiness">,
): string {
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

const EVENT_TYPES = {
	simple_chat: (t: DevicesT) => t("devices:eligibility.eventType.chat", "Chat"),
	page: (t: DevicesT) => t("devices:eligibility.eventType.page", "Page"),
	http: (t: DevicesT) => t("devices:eligibility.eventType.http", "Web request"),
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
		t("devices:eligibility.eventType.chatBot", "Chat bot"),
	telegram: (t: DevicesT) =>
		t("devices:eligibility.eventType.chatBot", "Chat bot"),
	generic_form: (t: DevicesT) =>
		t("devices:eligibility.eventType.form", "Form"),
	quick_action: (t: DevicesT) =>
		t("devices:eligibility.eventType.quickAction", "Quick action"),
	deeplink: (t: DevicesT) =>
		t("devices:eligibility.eventType.deeplink", "Deep link"),
	geolocation: (t: DevicesT) =>
		t("devices:eligibility.eventType.geolocation", "Location region"),
	api: (t: DevicesT) => t("devices:eligibility.eventType.api", "Endpoint"),
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
