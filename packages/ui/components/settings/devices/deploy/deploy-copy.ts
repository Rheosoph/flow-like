import { formatMoment } from "../../../../lib/date";
import type { BotProvider } from "../../../../lib/device-management/bot-config";
import {
	type EventKind,
	HUB_EXPORT_TYPES,
	type LatestFlowCase,
	eventKind,
} from "../../../../lib/device-management/deployment";
import type {
	LatestProblem,
	PreparedFlow,
} from "../../../../lib/device-management/latest-flows";
import type { AppMode } from "../../../../lib/device-management/model/app-plan";
import type {
	DeployPlan,
	PlanAcknowledgement,
	PlanException,
	PlanFacts,
	PlanIssue,
	PlanIssueCode,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	AgentFeature,
	CopyParams,
	DeployRoute,
	DeployStepId,
	DevicesRoute,
	DevicesScope,
	GateNoticeKind,
} from "../../../../lib/device-management/model/types";
import { agentTooOldCopy, eligibilityCopy } from "../copy/eligibility-copy";
import { type CopyFormat, gateCopy } from "../copy/gate-copy";
import type { DevicesT } from "../primitives/area-context";
import type { WhereGate } from "./deploy-facts";
import type { PrepareFailure } from "./use-deploy-prepare";

/* Copy of the deploy wizard's frame and steps 1–5 (APP §3, §7): step names and the sentence behind every plan check code. */

/** The eight steps of one entry; step 6 follows the app's mode (A1). */
export function deploySteps(mode: AppMode | null): DeployStepId[] {
	return [
		"what",
		"how",
		"where",
		"settings",
		"endpoint",
		mode === "offline" ? "copy_upload" : "access_cost",
		"review",
		"rollout",
	];
}

/** Review: the last step the frame's Back / Continue foot leads to. */
export const REVIEW_INDEX = 6;

/**
 * The step a route opens: its own, this mode's step 6 when it carries the
 * other mode's, the furthest step of a resumed draft, else What.
 */
export function currentStep(
	route: Pick<DeployRoute, "step">,
	steps: readonly DeployStepId[],
	saved: { resumed: boolean; reached: number },
): DeployStepId {
	const wanted = route.step;
	if (wanted && steps.includes(wanted)) return wanted;
	if (wanted === "access_cost" || wanted === "copy_upload") return steps[5];
	return saved.resumed ? steps[Math.min(saved.reached, REVIEW_INDEX)] : "what";
}

/** What the entry filled in: shown as done, still editable. */
export function prefilledSteps(
	route: Pick<DeployRoute, "appId" | "deviceIds">,
	scope: DevicesScope,
): Set<DeployStepId> {
	const steps = new Set<DeployStepId>();
	if (scope.kind === "app" || route.appId) steps.add("what");
	if (route.deviceIds.length) steps.add("where");
	return steps;
}

/** Where Exit leads inside the area (`deployExitHref` as a route); the Events page of `from=events` is another page and stays a link. */
export function exitRoute(
	route: Pick<DeployRoute, "deviceIds" | "serviceId">,
	scope: DevicesScope,
): DevicesRoute {
	if (scope.kind === "app") return { screen: "app-devices", by: "device" };
	const [deviceId] = route.deviceIds;
	if (!deviceId || route.deviceIds.length > 1)
		return { screen: "fleet", view: "devices" };
	return route.serviceId
		? { screen: "service", deviceId, serviceId: route.serviceId }
		: { screen: "device", deviceId, tab: "services" };
}

const STEP_LABELS = {
	what: (t) => t("devices:deploy.step.what", "What"),
	how: (t) => t("devices:deploy.step.how", "How it runs"),
	where: (t) => t("devices:deploy.step.where", "Where"),
	settings: (t) => t("devices:deploy.step.settings", "Settings"),
	endpoint: (t) => t("devices:deploy.step.endpoint", "Endpoint & limits"),
	access_cost: (t) => t("devices:deploy.step.accessCost", "Access & cost"),
	copy_upload: (t) => t("devices:deploy.step.copyUpload", "Copy & upload"),
	review: (t) => t("devices:deploy.step.review", "Review"),
	rollout: (t) => t("devices:deploy.step.rollout", "Rollout"),
} satisfies Record<DeployStepId, (t: DevicesT) => string>;

/** Stepper label; step 5 reads "Limits" when no chosen event is served by the device. */
export function stepLabel(
	t: DevicesT,
	step: DeployStepId,
	limitsOnly = false,
): string {
	if (step === "endpoint" && limitsOnly)
		return t("devices:deploy.step.limits", "Limits");
	return STEP_LABELS[step](t);
}

/** The step's heading and summary label. */
export function stepTitle(
	t: DevicesT,
	step: DeployStepId,
	limitsOnly = false,
): string {
	if (step === "what") return t("devices:deploy.title.what", "What to run");
	if (step === "where") return t("devices:deploy.title.where", "Where it runs");
	return stepLabel(t, step, limitsOnly);
}

interface GateContext {
	t: DevicesT;
	gate: WhereGate;
	device: string;
	service: string;
	fmt: CopyFormat;
}

/** The deploy-specific sentences of APP §3.7; a gate without one keeps the shared gate sentence. */
const WHERE_GATE_COPY: Partial<
	Record<WhereGate["code"], (c: GateContext) => string>
> = {
	never: ({ t, device }) =>
		t(
			"devices:deploy.gate.never",
			"{{device}} hasn't checked in yet. Deploying needs a live connection.",
			{ device },
		),
	offline: ({ t, gate, device, fmt }) => {
		const since = gate.failure.copy.params?.since;
		return typeof since === "number"
			? t(
					"devices:deploy.gate.offlineSince",
					"{{device}} is offline since {{since}}. Deploying needs a live connection.",
					{ device, since: fmt.at(since) },
				)
			: t(
					"devices:deploy.gate.offline",
					"{{device}} is offline. Deploying needs a live connection.",
					{ device },
				);
	},
	nokeys: ({ t, device }) =>
		t(
			"devices:deploy.gate.nokeys",
			"This computer has no keys for {{device}}.",
			{
				device,
			},
		),
	busy: ({ t, gate, service, fmt }) =>
		gate.by === undefined
			? t(
					"devices:deploy.gate.busy",
					"Wait for {{service}}'s update to finish.",
					{
						service,
					},
				)
			: t(
					"devices:deploy.gate.busyBy",
					"Wait for {{service}}'s update to finish (by {{by}} at the latest).",
					{ service, by: fmt.at(gate.by) },
				),
	staged: ({ t, service }) =>
		t(
			"devices:deploy.gate.staged",
			"An update for {{service}} is already staged. Activate or discard it first.",
			{ service },
		),
};

/** Why a device can't take a deploy right now (APP §3.7 table). */
export function whereGateText(
	t: DevicesT,
	gate: WhereGate,
	device: string,
	fmt: CopyFormat,
): string {
	const copy = WHERE_GATE_COPY[gate.code];
	return copy
		? copy({ t, gate, device, service: gate.serviceId ?? "", fmt })
		: gateCopy(t, gate.failure, fmt).inline;
}

const WHERE_GATE_KIND: Partial<Record<WhereGate["code"], GateNoticeKind>> = {
	never: "live",
	offline: "live",
	nokeys: "nokeys",
	busy: "busy",
	staged: "busy",
};

/** The notice kind (icon) of a Where gate. */
export function whereGateKind(gate: WhereGate): GateNoticeKind {
	return WHERE_GATE_KIND[gate.code] ?? gate.failure.kind;
}

/** Why an event that follows Latest can't be shipped now; the Latest cases share their sentences with the event rule. */
function latestFailureText(
	t: DevicesT,
	problem: LatestProblem,
	failure: PrepareFailure,
	event: string,
): string {
	const reason = (latestFlow: LatestFlowCase) =>
		eligibilityCopy(t, { code: "latest_flow", eventType: "", latestFlow }).long;
	if (problem === "busy")
		return t(
			"devices:deploy.prepare.flowBusy",
			"Someone is editing this flow right now. Try again in a moment.",
		);
	if (problem === "moved")
		return t(
			"devices:deploy.prepare.flowMoved",
			"{{event}}'s flow changed while preparing. Prepare again.",
			{ event },
		);
	if (problem === "incomparable")
		return (
			failure.detail ||
			t(
				"devices:deploy.prepare.flowIncomparable",
				"This flow can't be compared with its published version. Pin a flow version in Events to deploy this event.",
			)
		);
	return reason(problem);
}

/** Why the device's agent got nothing: the flag an event needs, as the end of "Failed while …: …". */
const AGENT_FEATURE_REFUSALS: Partial<
	Record<AgentFeature, (t: DevicesT) => string>
> = {
	api_events: (t) =>
		t(
			"devices:deployShip.fail.agentFeature.apiEvents",
			"the device's agent is too old to serve Endpoints",
		),
	scheduled_events: (t) =>
		t(
			"devices:deployShip.fail.agentSchedules",
			"the device's agent is too old to run schedules",
		),
	scheduled_once: (t) =>
		t(
			"devices:deployShip.fail.agentFeature.scheduledOnce",
			"the device's agent is too old to run one-time schedules",
		),
	on_demand_events: (t) =>
		t(
			"devices:deployShip.fail.agentFeature.onDemandEvents",
			"the device's agent is too old to run forms and quick actions",
		),
	telegram_bots: (t) =>
		t(
			"devices:deployShip.fail.agentFeature.telegramBots",
			"the device's agent is too old to run Telegram bots",
		),
	discord_bots: (t) =>
		t(
			"devices:deployShip.fail.agentFeature.discordBots",
			"the device's agent is too old to run Discord bots",
		),
	model_store: (t) =>
		t(
			"devices:deployShip.models.fail.agent",
			"the device's agent is too old to fetch model files itself",
		),
};

/** Why the step that gets a version's model files onto the device stopped; `file` names the file. */
const MODEL_REFUSALS: Record<string, (t: DevicesT, file: string) => string> = {
	model_disk: (t, file) =>
		t(
			"devices:deployShip.models.fail.disk",
			"the device's model disk has no room for {{file}}",
			{ file },
		),
	model_unreachable: (t, file) =>
		t(
			"devices:deployShip.models.fail.unreachable",
			"the device couldn't download {{file}}, and this version of the app can't send it from here",
			{ file },
		),
	model_push: (t, file) =>
		t(
			"devices:deployShip.models.fail.push",
			"sending {{file}} from this computer failed",
			{ file },
		),
	model_push_cancelled: (t, file) =>
		t(
			"devices:deployShip.models.fail.stopped",
			"you stopped sending {{file}}",
			{ file },
		),
	model_reply: (t) =>
		t(
			"devices:deployShip.models.fail.reply",
			"the device's answer about its model files was invalid",
		),
};

function agentFeatureRefusal(t: DevicesT, feature: string): string {
	const copy = Object.hasOwn(AGENT_FEATURE_REFUSALS, feature)
		? AGENT_FEATURE_REFUSALS[feature as AgentFeature]
		: undefined;
	return copy
		? copy(t)
		: t(
				"devices:deployShip.fail.agentFeature.other",
				"the device's agent is too old for one of its events",
			);
}

/**
 * Why a run stopped a target, or its preparation, over a schedule, a bot, an
 * agent flag or an event that follows Latest: the end of "Failed while …: …".
 * `detail` is what the run named with the failure (the event a refusal is
 * about, the flag an agent lacks).
 */
export const RUN_REFUSALS: Record<
	string,
	(t: DevicesT, detail: string) => string
> = {
	...MODEL_REFUSALS,
	agent_feature: agentFeatureRefusal,
	bot_role: (t, event) =>
		t(
			"devices:deployShip.fail.botRole",
			"moving {{event}} to a device needs the right to edit this app's events",
			{ event },
		),
	bot_elsewhere: (t, event) =>
		t(
			"devices:deployShip.fail.botElsewhere",
			"{{event}} is already assigned to another service, and a bot runs in one place",
			{ event },
		),
	bot_returning: (t, event) =>
		t(
			"devices:deployShip.fail.botReturning",
			"{{event}} is still being given up where it ran",
			{ event },
		),
	bot_hub: (t) =>
		t(
			"devices:deployShip.fail.botHub",
			"this hub can't hand bots to devices yet",
		),
	hub_type: (t, event) =>
		t(
			"devices:deployShip.fail.hubType",
			"this hub can't hand {{event}} to devices yet; update the hub",
			{ event },
		),
	event_missing: (t, event) =>
		t(
			"devices:deployShip.fail.eventMissing",
			"{{event}} isn't in what the hub publishes for devices",
			{ event },
		),
	schedule_role: (t, event) =>
		t(
			"devices:deployShip.fail.scheduleRole",
			"moving {{event}} off the hub needs the right to edit this app's events",
			{ event },
		),
	schedule_elsewhere: (t, event) =>
		t(
			"devices:deployShip.fail.scheduleElsewhere",
			"{{event}} is already assigned to another service, and a schedule runs in one place",
			{ event },
		),
	schedule_returning: (t, event) =>
		t(
			"devices:deployShip.fail.scheduleReturning",
			"{{event}} is still returning to the hub from where it ran",
			{ event },
		),
	schedule_hub: (t) =>
		t(
			"devices:deployShip.fail.scheduleHub",
			"this hub can't run schedules on devices yet",
		),
	agent_schedules: (t) =>
		t(
			"devices:deployShip.fail.agentSchedules",
			"the device's agent is too old to run schedules",
		),
	flow_busy: (t) =>
		t(
			"devices:deployShip.fail.flowBusy",
			"someone is editing one of its flows right now",
		),
	flow_role: (t) =>
		t(
			"devices:deployShip.fail.flowRole",
			"a flow has edits that aren't published as a version, and your role can't create one",
		),
	flow_hub: (t) =>
		t(
			"devices:deployShip.fail.flowHub",
			"this hub can't deploy events that follow Latest yet",
		),
	flow_moved: (t) =>
		t(
			"devices:deployShip.fail.flowMoved",
			"a flow changed while this was being prepared",
		),
	flow_target: (t) =>
		t(
			"devices:deployShip.fail.flowTarget",
			"an event points at a Page or a start node that is no longer in its flow",
		),
	flow_incomparable: (t) =>
		t(
			"devices:deployShip.fail.flowIncomparable",
			"a flow can't be compared with its published version",
		),
};

/** The sentence for one of those refusals; null for a failure the device or the hub words itself. */
export function runRefusalText(
	t: DevicesT,
	error: { code: string; detail?: string },
): string | null {
	return Object.hasOwn(RUN_REFUSALS, error.code)
		? RUN_REFUSALS[error.code](t, error.detail ?? "")
		: null;
}

/** How a sentence names a flow: by its own name, else by an event that follows it. */
export function flowLabel(
	t: DevicesT,
	plan: DeployPlan,
	names: ReadonlyMap<string, string>,
	boardId: string,
): string {
	const known = names.get(boardId);
	if (known) return known;
	const event = plan.app?.events.find((row) => row.boardId === boardId);
	return t("devices:deploy.prepare.flowOf", "the flow of {{event}}", {
		event: event?.name ?? boardId,
	});
}

/** What preparing did for one flow of the events that follow Latest. */
export function preparedFlowText(
	t: DevicesT,
	flow: PreparedFlow,
	name: string,
): string {
	const values = { version: flow.version.join("."), flow: name };
	return flow.created
		? t(
				"devices:deploy.prepare.flowCreated",
				"Created flow version {{version}} of {{flow}}",
				values,
			)
		: t(
				"devices:deploy.prepare.flowUnchanged",
				"Nothing changed in {{flow}} since {{version}}",
				values,
			);
}

/**
 * An event that a hub exports only when asked for its type, from a hub whose
 * placement list names the types it hands to devices and not this one.
 */
export function hubLacksExport(
	event: { event_type: string; default_page_id?: string | null } | undefined,
	hubTypes: readonly string[] | undefined,
): boolean {
	return (
		!!event &&
		hubTypes !== undefined &&
		!event.default_page_id &&
		(HUB_EXPORT_TYPES as readonly string[]).includes(event.event_type) &&
		!hubTypes.includes(event.event_type)
	);
}

/** A chosen event the hub's bundle doesn't carry: the hub's fault when it can't export its type, else the event's. */
function missingEventText(
	t: DevicesT,
	plan: DeployPlan,
	eventId: string,
	hubTypes: readonly string[] | undefined,
) {
	const record = plan.app?.events.find((row) => row.id === eventId);
	if (record && hubLacksExport(record, hubTypes))
		return eligibilityCopy(t, {
			code: "hub_type",
			eventType: record.event_type,
		}).long;
	return t(
		"devices:deploy.prepare.eventMissing",
		"{{event}} isn't in what the hub publishes for devices. Check it in Events, then prepare again.",
		{ event: eventName(plan, eventId) },
	);
}

/** A chosen event the hub refused or left out of its bundle. */
function eventFailureText(
	t: DevicesT,
	failure: PrepareFailure,
	plan: DeployPlan,
	hubTypes: readonly string[] | undefined,
) {
	const eventId = failure.eventId ?? "";
	return failure.detail
		? t(
				"devices:deploy.prepare.eventBlocked",
				"{{event}} can't be prepared for devices: {{detail}}",
				{ event: eventName(plan, eventId), detail: failure.detail },
			)
		: missingEventText(t, plan, eventId, hubTypes);
}

/**
 * The failing preparation check's sentence (APP §3.6 item 3). `hubTypes` is
 * the hub's `event_types`: a chosen event of a type that hub can't export is
 * missing because of the hub, not because of the event.
 */
export function prepareFailureText(
	t: DevicesT,
	failure: PrepareFailure,
	plan: DeployPlan,
	hubTypes?: readonly string[],
): string {
	if (failure.kind === "flow")
		return latestFailureText(
			t,
			failure.flow ?? "moved",
			failure,
			eventName(plan, failure.eventId ?? ""),
		);
	return failure.kind === "event"
		? eventFailureText(t, failure, plan, hubTypes)
		: failure.detail;
}

export interface IssueNames {
	app: string;
	device(deviceId: string | undefined): string;
	event(eventId: string): string;
	/** A unix-seconds time in the reader's words; the default locale without it. */
	at?(atS: number): string;
	/** How a device runs the event; unknown without it. */
	kind?(eventId: string): EventKind | null;
}

interface IssueContext {
	t: DevicesT;
	p: CopyParams;
	device: string;
	app: string;
	issue: PlanIssue;
	names: IssueNames;
	/** The event a schedule, bot or route issue is about. */
	event: string;
}

const momentOf = (names: IssueNames, atS: unknown) =>
	typeof atS === "number" ? (names.at?.(atS) ?? formatMoment(atS * 1000)) : "";

const PROVIDER_NAMES: Record<BotProvider, (t: DevicesT) => string> = {
	telegram: (t) => t("devices:deploy.provider.telegram", "Telegram"),
	discord: (t) => t("devices:deploy.provider.discord", "Discord"),
};

/** A bot's provider by its name ("Telegram"). */
export function providerName(t: DevicesT, provider: unknown): string {
	return typeof provider === "string" && Object.hasOwn(PROVIDER_NAMES, provider)
		? PROVIDER_NAMES[provider as BotProvider](t)
		: t("devices:deploy.provider.other", "the provider");
}

/** What a person confirms per device before Continue, as the step's blocking sentence. */
const ACKNOWLEDGE_COPY: Record<
	PlanAcknowledgement,
	(t: DevicesT, device: string) => string
> = {
	once_soon: (t, device) =>
		t(
			"devices:deploy.issue.acknowledge.onceSoon",
			"Confirm on {{device}} that a one-time schedule runs in less than 5 minutes, or set a later time in Events.",
			{ device },
		),
	bot_open: (t, device) =>
		t(
			"devices:deploy.issue.acknowledge.botOpen",
			"Confirm on {{device}} that anyone who can message its bot can start runs, or set allowed chats in Events.",
			{ device },
		),
	bot_other_computers: (t, device) =>
		t(
			"devices:deploy.issue.acknowledge.botOtherComputers",
			"Confirm on {{device}} that no other computer keeps running its bot in the desktop app.",
			{ device },
		),
	endpoint_shared_token: (t, device) =>
		t(
			"devices:deploy.issue.acknowledge.endpointSharedToken",
			"Confirm on {{device}} that the Endpoint uses the service's access settings. Its token from Events is not used.",
			{ device },
		),
};

function acknowledgeText({ t, p, device }: IssueContext): string {
	const code = String(p.exception ?? "");
	return Object.hasOwn(ACKNOWLEDGE_COPY, code)
		? ACKNOWLEDGE_COPY[code as PlanAcknowledgement](t, device)
		: t(
				"devices:deploy.issue.acknowledge.other",
				"Confirm the warnings on {{device}} first.",
				{ device },
			);
}

const isBotIssue = ({ names, p }: IssueContext) =>
	typeof p.event === "string" && names.kind?.(p.event) === "bot";

const ISSUE_COPY = {
	app_missing: ({ t }) =>
		t("devices:deploy.issue.appMissing", "Choose an app first."),
	local_only_web: ({ t }) =>
		t(
			"devices:deploy.issue.localOnlyWeb",
			"Offline copies are prepared by the desktop app.",
		),
	no_events: ({ t }) =>
		t("devices:deploy.issue.noEvents", "Pick at least one event."),
	too_many_events: ({ t }) =>
		t(
			"devices:deploy.issue.tooManyEvents",
			"A service can serve 64 events at most.",
		),
	service_id_invalid: ({ t }) =>
		t(
			"devices:deploy.issue.serviceIdInvalid",
			"A service ID has 1 to 128 letters, digits, dots, dashes or underscores.",
		),
	service_id_twice: ({ t, p, device, issue }) =>
		issue.deviceId
			? t(
					"devices:deploy.issue.serviceTwiceOnDevice",
					"Two services of this deploy would both become {{service}} on {{device}}. Pick another one for one of them.",
					{ service: p.service, device },
				)
			: t(
					"devices:deploy.issue.serviceIdTwice",
					"Each service needs its own ID.",
				),
	service_id_reserved: ({ t, p, device }) =>
		t(
			"devices:deploy.issue.serviceIdReserved",
			"{{service}} was used before on {{device}} and can't be reused.",
			{ service: p.service, device },
		),
	removed_events: ({ t }) =>
		t(
			"devices:deploy.issue.removedEvents",
			"Confirm the events that stop being served after the update.",
		),
	no_targets: ({ t }) =>
		t("devices:deploy.issue.noTargets", "Pick at least one device."),
	target_gated: ({ t, device }) =>
		t(
			"devices:deploy.issue.targetGated",
			"{{device}} can't take a deploy right now.",
			{ device },
		),
	target_locked: ({ t, device }) =>
		t("devices:deploy.issue.targetLocked", "Unlock {{device}} to continue.", {
			device,
		}),
	target_no_events: ({ t, device }) =>
		t(
			"devices:deploy.issue.targetNoEvents",
			"None of the picked events can run on {{device}}.",
			{ device },
		),
	variable_invalid: ({ t, p, device }) =>
		t(
			"devices:deploy.issue.variableInvalid",
			"{{device}}: {{variable}} isn't a valid value.",
			{ device, variable: p.variable },
		),
	secret_size: ({ t, p, device }) =>
		t(
			"devices:deploy.issue.secretSize",
			"{{device}}: {{variable}} needs 1 to 4,096 bytes.",
			{ device, variable: p.variable },
		),
	service_instances_range: ({ t }) =>
		t("devices:deploy.issue.instancesRange", "Instances go from 1 to 32."),
	update_needs_service: ({ t, device }) =>
		t(
			"devices:deploy.issue.updateNeedsService",
			"Pick the service to update on {{device}}.",
			{ device },
		),
	host_invalid: ({ t, device }) =>
		t(
			"devices:deploy.issue.hostInvalid",
			"{{device}}: enter an IP address, not a host name.",
			{ device },
		),
	port_invalid: ({ t, device }) =>
		t(
			"devices:deploy.issue.portInvalid",
			"{{device}}: enter a port from 1 to 65535.",
			{ device },
		),
	port_in_use: ({ t, p, device }) =>
		p.service === undefined
			? t(
					"devices:deploy.issue.portInUse",
					"Port {{port}} is already in use on {{device}}.",
					{ port: String(p.port), device },
				)
			: t(
					"devices:deploy.issue.portInUseBy",
					"Port {{port}} is used by {{service}} on {{device}}.",
					{ port: String(p.port), service: p.service, device },
				),
	token_invalid: ({ t }) =>
		t(
			"devices:deploy.issue.tokenInvalid",
			"The access token needs at least 32 printable characters without spaces.",
		),
	unencrypted: ({ t, device }) =>
		t(
			"devices:deploy.issue.unencrypted",
			"Confirm serving unencrypted on {{device}}, or pick a certificate there.",
			{ device },
		),
	writes_offline: ({ t }) =>
		t(
			"devices:deploy.issue.writesOffline",
			"Write buffering is for online apps only.",
		),
	writes_invalid: ({ t }) =>
		t(
			"devices:deploy.issue.writesInvalid",
			"Check the tables, folders and budgets of write buffering.",
		),
	isolation_unavailable: ({ t, device }) =>
		t(
			"devices:deploy.issue.isolationUnavailable",
			"{{device}} can't sandbox services. Choose Runs as the agent for it.",
			{ device },
		),
	isolation_required: ({ t, device }) =>
		t(
			"devices:deploy.issue.isolationRequired",
			"{{device}} requires sandboxed services.",
			{ device },
		),
	agent_trust: ({ t, device, app }) =>
		t(
			"devices:deploy.issue.agentTrust",
			"Confirm that {{device}} runs {{app}} with the agent's full access.",
			{ device, app },
		),
	schedule_elsewhere: ({ t, p, names, event }) =>
		typeof p.device === "string" && p.service !== undefined
			? t(
					"devices:deploy.issue.scheduleElsewhere",
					"{{event}} is already assigned to {{device}} › {{service}}. A schedule runs in one place: remove it there first, or run it on the hub again in Events.",
					{ event, device: names.device(p.device), service: p.service },
				)
			: t(
					"devices:deploy.issue.scheduleElsewhereHidden",
					"{{event}} is already assigned to a device you can't see. A schedule runs in one place: run it on the hub again in Events first.",
					{ event },
				),
	schedule_returning: ({ t, p, names, event }) =>
		t(
			"devices:deploy.issue.scheduleReturning",
			"{{event}} is still returning to the hub from where it ran. Deploy it after {{time}}.",
			{ event, time: momentOf(names, p.time) },
		),
	schedule_role: ({ t, event }) =>
		t(
			"devices:deploy.issue.scheduleRole",
			"Moving {{event}} off the hub needs the right to edit this app's events. Ask someone who has it, or leave the schedule out.",
			{ event },
		),
	schedule_twice: ({ t, device }) =>
		t(
			"devices:deploy.issue.scheduleTwice",
			"Confirm on {{device}} that its schedule may run in more than one place, or leave it out.",
			{ device },
		),
	needs_single_instance: (c) =>
		isBotIssue(c)
			? c.t(
					"devices:deploy.issue.needsSingleInstanceBot",
					"{{service}} runs {{count, number}} instances. A bot needs a service with 1 instance: set it to 1, or deploy the bot as its own service.",
					{ service: c.p.service, count: Number(c.p.count) },
				)
			: c.t(
					"devices:deploy.issue.needsSingleInstance",
					"{{service}} runs {{count, number}} instances. A schedule needs a service with 1 instance: set it to 1, or deploy the schedule as its own service.",
					{ service: c.p.service, count: Number(c.p.count) },
				),
	too_many_latest: ({ t, p }) =>
		t(
			"devices:deploy.issue.tooManyLatest",
			"This deploy has {{count, number}} events that follow Latest; one deploy can take 64. Deploy fewer at a time, or pin some in Events.",
			{ count: Number(p.count) },
		),
	route_conflict: ({ t, p, names, event }) =>
		t(
			"devices:deploy.issue.routeConflict",
			"{{a}} and {{b}} both answer {{method}} {{path}}. A service answers each path once: change one in Events, or deploy them as two services.",
			{
				a: event,
				b: typeof p.other === "string" ? names.event(p.other) : "",
				method: p.method,
				path: p.path,
			},
		),
	once_passed: ({ t, p, names, event }) =>
		t(
			"devices:deploy.issue.oncePassed",
			"{{event}}'s time ({{time}}) has passed. Set a new time in Events, or leave it out.",
			{ event, time: momentOf(names, p.time) },
		),
	bot_token_missing: ({ t, event }) =>
		t(
			"devices:deploy.issue.botToken",
			"{{event}} needs its bot token. Enter it under Settings.",
			{ event },
		),
	bot_token_shape: ({ t, p }) =>
		t(
			"devices:deploy.issue.botTokenShape",
			"That doesn't look like a {{provider}} bot token.",
			{ provider: providerName(t, p.provider) },
		),
	bot_local_trigger: ({ t, event }) =>
		t(
			"devices:deploy.issue.botLocalTrigger",
			"This computer runs {{event}} while Flow-Like is open. Two programs can't use one bot: stop it here first.",
			{ event },
		),
	bot_elsewhere: ({ t, p, names, event }) =>
		typeof p.device === "string" && p.service !== undefined
			? t(
					"devices:deploy.issue.botElsewhere",
					"{{event}} is already assigned to {{device}} › {{service}}. A bot runs in one place: remove it there first, or take it back in Events.",
					{ event, device: names.device(p.device), service: p.service },
				)
			: t(
					"devices:deploy.issue.botElsewhereHidden",
					"{{event}} is already assigned to a device you can't see. A bot runs in one place: take it back in Events first.",
					{ event },
				),
	bot_returning: ({ t, p, names, event }) =>
		t(
			"devices:deploy.issue.botReturning",
			"{{event}} is still being given up where it ran. Deploy it after {{time}}.",
			{ event, time: momentOf(names, p.time) },
		),
	bot_role: ({ t, event }) =>
		t(
			"devices:deploy.issue.botRole",
			"Moving {{event}} to a device needs the right to edit this app's events. Ask someone who has it, or leave the bot out.",
			{ event },
		),
	too_many_claimed: ({ t, p }) =>
		t(
			"devices:deploy.issue.tooManyClaimed",
			"{{service}} would run {{count, number}} schedules and bots. One service runs 64 at most: split them over two services.",
			{ service: p.service, count: Number(p.count) },
		),
	form_needs_page: ({ t, event }) =>
		t(
			"devices:deploy.issue.formNeedsPage",
			"{{event}} takes a file. Files can only be sent from a service page: deploy it together with a Page, chat or Endpoint.",
			{ event },
		),
	not_acknowledged: acknowledgeText,
	"approval.models_invalid": ({ t }) =>
		t("devices:deploy.issue.approvalModels", "Pick up to 64 different models."),
	"approval.nothing_approved": ({ t }) =>
		t(
			"devices:deploy.issue.approvalNothing",
			"Approve project files or at least one model.",
		),
	"approval.files_need_app": ({ t }) =>
		t(
			"devices:deploy.issue.approvalFilesApp",
			"Project files need an online app.",
		),
	"approval.files_need_owner": ({ t, app }) =>
		t(
			"devices:deploy.issue.approvalFilesOwner",
			"Only the owner of {{app}} can approve access to its files.",
			{ app },
		),
	"approval.files_need_consent": ({ t, app }) =>
		t(
			"devices:deploy.issue.approvalFilesConsent",
			"Confirm that these services may use the files of {{app}}.",
			{ app },
		),
	"approval.instances_range": ({ t }) =>
		t(
			"devices:deploy.issue.approvalInstancesRange",
			"Allowed instances go from 1 to 100.",
		),
	"approval.instances_below_service": ({ t, p }) =>
		t(
			"devices:deploy.issue.approvalInstancesBelow",
			"Allow at least {{count, number}} instances: the service runs that many.",
			{ count: Number(p.count) },
		),
	"approval.expiry_past": ({ t }) =>
		t(
			"devices:deploy.issue.approvalExpiryPast",
			"Cloud access must end in the future.",
		),
	"approval.expiry_too_far": ({ t }) =>
		t(
			"devices:deploy.issue.approvalExpiryFar",
			"Cloud access can last 365 days at most.",
		),
	"approval.spending_required": ({ t }) =>
		t(
			"devices:deploy.issue.spendingRequired",
			"Set a spending limit: approved models need one.",
		),
	"approval.spending_without_models": ({ t }) =>
		t(
			"devices:deploy.issue.spendingWithoutModels",
			"A spending limit needs at least one model.",
		),
	"approval.spending_range": ({ t }) =>
		t(
			"devices:deploy.issue.spendingRange",
			"Enter a spending limit above zero.",
		),
	"approval.spending_expiry": ({ t }) =>
		t(
			"devices:deploy.issue.spendingExpiry",
			"The spending limit can't outlast the cloud access.",
		),
	"approval.spending_consent": ({ t }) =>
		t(
			"devices:deploy.issue.spendingConsent",
			"Confirm that you pay for model use up to the limit.",
		),
} satisfies Record<PlanIssueCode, (c: IssueContext) => string>;

/** The sentence behind a plan check code (R3: codes never reach the screen). */
export function issueText(
	t: DevicesT,
	issue: PlanIssue,
	names: IssueNames,
): string {
	const p = issue.params ?? {};
	return ISSUE_COPY[issue.code]({
		t,
		p,
		device: names.device(issue.deviceId),
		app: names.app,
		issue,
		names,
		event: typeof p.event === "string" ? names.event(p.event) : "",
	});
}

/**
 * Device, event and app names of one plan, for `issueText` and
 * `exceptionText`. `facts` names devices that are not targets (where a
 * schedule runs today); `at` formats times in the area's locale.
 */
export function planNames(
	t: DevicesT,
	plan: DeployPlan,
	facts?: Pick<PlanFacts, "devices">,
	at?: IssueNames["at"],
): IssueNames {
	return {
		app: plan.app?.name ?? t("devices:deploy.thisApp", "this app"),
		device: (deviceId) =>
			deviceNameOf(plan, facts, deviceId) ??
			t("devices:deploy.aDevice", "a device"),
		event: (eventId) => eventName(plan, eventId),
		kind: (eventId) => eventKindOf(plan, eventId),
		...(at ? { at } : {}),
	};
}

/** A target's name, else the name of a device the plan's facts know. */
function deviceNameOf(
	plan: DeployPlan,
	facts: Pick<PlanFacts, "devices"> | undefined,
	deviceId: string | undefined,
) {
	const isIt = (target: { deviceId: string }) => target.deviceId === deviceId;
	const target = plan.targets.find(isIt);
	if (target) return target.name;
	return deviceId ? facts?.devices[deviceId]?.name : undefined;
}

function eventKindOf(plan: DeployPlan, eventId: string) {
	const isIt = (row: { id: string }) => row.id === eventId;
	const event = plan.app?.events.find(isIt);
	return event ? eventKind(event) : null;
}

/** Why an app can't be deployed by a role without Read boards (APP §3.5 item 1). */
export const noFlowsText = (t: DevicesT, app: string): string =>
	t(
		"devices:deploy.what.noFlowsHint",
		"Your role can't read the flows of {{app}}.",
		{ app },
	);

export function eventName(plan: DeployPlan, eventId: string): string {
	return (
		plan.app?.events.find((event) => event.id === eventId)?.name ?? eventId
	);
}

export interface ExceptionText {
	/** The "Differs" cell. */
	differs: string;
	/** The "Why" cell. */
	why: string;
}

interface ExceptionContext {
	t: DevicesT;
	p: CopyParams;
	device: string;
	event: string;
	/** `port_moved`: the service that holds the port the plan asked for. */
	usedBy?: string;
	/** `runs_as_agent`: the device has no sandbox, so nobody chose it. */
	cantSandbox?: boolean;
	/** A unix-seconds time in the reader's words. */
	at(atS: unknown): string;
}

const scheduleAlso = (t: DevicesT, event: string) =>
	t("devices:deploy.exception.scheduleAlso", "{{event}} · runs elsewhere too", {
		event,
	});

const EXCEPTION_COPY = {
	left_out_refuse: ({ t, p, event }) => ({
		differs: t("devices:deploy.exception.leftOut", "{{event}} · left out", {
			event,
		}),
		why: t("devices:deploy.exception.refuseWhy", "Can't run here: {{reason}}", {
			reason: p.reason,
		}),
	}),
	left_out_agent: ({ t, p, device, event }) => ({
		differs: t("devices:deploy.exception.leftOut", "{{event}} · left out", {
			event,
		}),
		why: agentTooOldCopy(
			t,
			device,
			typeof p.feature === "string" ? p.feature : undefined,
		).long,
	}),
	once_soon: ({ t, p, event, at }) => ({
		differs: t(
			"devices:deploy.exception.onceSoonDiffers",
			"{{event}} · runs very soon",
			{ event },
		),
		why: t(
			"devices:deploy.exception.onceSoon",
			"{{event}} runs at {{time}}, in less than 5 minutes. If the deploy isn't finished by then, it doesn't run.",
			{ event, time: at(p.time) },
		),
	}),
	bot_open: ({ t, device, event }) => ({
		differs: t(
			"devices:deploy.exception.botOpenDiffers",
			"{{event}} · open to everyone",
			{ event },
		),
		why: t(
			"devices:deploy.exception.botOpen",
			"Anyone who can message {{event}} can start runs on {{device}}. Allowed chats are set in Events.",
			{ event, device },
		),
	}),
	bot_other_computers: ({ t, event }) => ({
		differs: t(
			"devices:deploy.exception.botOtherComputersDiffers",
			"{{event}} · may run elsewhere",
			{ event },
		),
		why: t(
			"devices:deploy.exception.botOtherComputers",
			"Other computers that run {{event}} in the desktop app can't be seen from here. Stop it there too.",
			{ event },
		),
	}),
	endpoint_shared_token: ({ t, p, device, event }) => ({
		differs: t(
			"devices:deploy.exception.endpointSharedTokenDiffers",
			"{{event}} · the service's access settings",
			{ event },
		),
		why: t(
			"devices:deploy.exception.endpointSharedToken",
			"{{event}} has its own token in Events. On {{device}} it uses {{service}}'s access settings, like every other endpoint, Page and chat of {{service}}. Its token from Events is not used.",
			{ event, device, service: p.service },
		),
	}),
	schedule_two_devices: ({ t, p, event }) => ({
		differs: scheduleAlso(t, event),
		why: t(
			"devices:deploy.exception.scheduleTwoDevices",
			"{{event}} will run on {{count, number}} devices. Each runs it on its own copy of the app's data; emails and other outside effects happen once per device.",
			{ event, count: Number(p.count) },
		),
	}),
	schedule_local_trigger: ({ t, event }) => ({
		differs: scheduleAlso(t, event),
		why: t(
			"devices:deploy.exception.scheduleLocalTrigger",
			"This computer also runs {{event}} while Flow-Like is open.",
			{ event },
		),
	}),
	left_out_duplicate: ({ t, p, event }) => ({
		differs: t("devices:deploy.exception.leftOut", "{{event}} · left out", {
			event,
		}),
		why: t(
			"devices:deploy.exception.duplicateWhy",
			"{{service}} already serves it here.",
			{ service: p.service },
		),
	}),
	renamed: ({ t, p, device }) => ({
		differs: t("devices:deploy.exception.renamed", "Service {{to}}", {
			to: p.to,
		}),
		why: t(
			"devices:deploy.exception.renamedWhy",
			"{{from}} is taken on {{device}}; this one becomes {{to}}.",
			{ from: p.from, to: p.to, device },
		),
	}),
	port_moved: ({ t, p, usedBy }) => ({
		differs: t("devices:deploy.exception.portMoved", "Port {{to}}", {
			to: String(p.to),
		}),
		why:
			p.service !== undefined
				? t(
						"devices:deploy.exception.portMovedService",
						"{{service}} of this deploy takes the next free port, {{to}}: one service per port.",
						{ service: p.service, to: String(p.to) },
					)
				: usedBy
					? t(
							"devices:deploy.exception.portMovedUsedBy",
							"Port {{from}} is used by {{service}} there, so it uses {{to}} instead.",
							{ from: String(p.from), service: usedBy, to: String(p.to) },
						)
					: t(
							"devices:deploy.exception.portMovedWhy",
							"Port {{from}} is in use there, so it uses {{to}} instead.",
							{ from: String(p.from), to: String(p.to) },
						),
	}),
	no_certificate: ({ t, device }) => ({
		differs: t(
			"devices:deploy.exception.noCertificate",
			"No certificate · unencrypted",
		),
		why: t(
			"devices:deploy.exception.noCertificateWhy",
			"No certificate is picked for {{device}}. Anyone on its network can read the traffic.",
			{ device },
		),
	}),
	runs_as_agent: ({ t, device, cantSandbox }) => ({
		differs: t("devices:deploy.exception.runsAsAgent", "Runs as the agent"),
		why: cantSandbox
			? t(
					"devices:deploy.exception.cantSandboxWhy",
					"{{device}} can't sandbox services, so it has the agent's full access there.",
					{ device },
				)
			: t(
					"devices:deploy.exception.runsAsAgentWhy",
					"It isn't sandboxed on {{device}}, so it has the agent's full access there.",
					{ device },
				),
	}),
} satisfies Record<
	PlanException["code"],
	(c: ExceptionContext) => ExceptionText
>;

/** The "Differs" and "Why" cells of an exceptions row (APP §3.7, §3.9). */
export function exceptionText(
	t: DevicesT,
	exception: PlanException,
	plan: DeployPlan,
	facts?: PlanFacts,
	at?: IssueNames["at"],
): ExceptionText {
	const p = exception.params ?? {};
	const device = facts?.devices[exception.deviceId];
	const usedBy = device?.portsInUse?.find(
		(row) => row.port === p.from,
	)?.serviceId;
	const names: IssueNames = {
		...planNames(t, plan, facts),
		...(at ? { at } : {}),
	};
	return EXCEPTION_COPY[exception.code]({
		t,
		p,
		device: names.device(exception.deviceId),
		event: typeof p.event === "string" ? eventName(plan, p.event) : "",
		at: (atS) => momentOf(names, atS),
		...(usedBy ? { usedBy } : {}),
		...(device?.isolation === "none" ? { cantSandbox: true } : {}),
	});
}

/** The fix "Deploy it as its own service" of an Endpoint that has its own token. */
export const ownServiceFixLabel = (t: DevicesT): string =>
	t(
		"devices:deploy.exception.endpointSharedTokenFix",
		"Deploy it as its own service",
	);
