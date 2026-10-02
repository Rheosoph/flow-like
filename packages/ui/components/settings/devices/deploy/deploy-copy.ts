import type { AppMode } from "../../../../lib/device-management/model/app-plan";
import type {
	DeployPlan,
	PlanException,
	PlanFacts,
	PlanIssue,
	PlanIssueCode,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	CopyParams,
	DeployRoute,
	DeployStepId,
	DevicesRoute,
	DevicesScope,
	GateNoticeKind,
} from "../../../../lib/device-management/model/types";
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

/** The failing preparation check's sentence (APP §3.6 item 3). */
export function prepareFailureText(
	t: DevicesT,
	failure: PrepareFailure,
	plan: DeployPlan,
): string {
	if (failure.kind !== "event") return failure.detail;
	const event = eventName(plan, failure.eventId ?? "");
	return failure.detail
		? t(
				"devices:deploy.prepare.eventBlocked",
				"{{event}} can't be prepared for devices: {{detail}}",
				{ event, detail: failure.detail },
			)
		: t(
				"devices:deploy.prepare.eventMissing",
				"{{event}} isn't in what the hub publishes for devices. Check it in Events, then prepare again.",
				{ event },
			);
}

export interface IssueNames {
	app: string;
	device(deviceId: string | undefined): string;
}

interface IssueContext {
	t: DevicesT;
	p: CopyParams;
	device: string;
	app: string;
	issue: PlanIssue;
}

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
	return ISSUE_COPY[issue.code]({
		t,
		p: issue.params ?? {},
		device: names.device(issue.deviceId),
		app: names.app,
		issue,
	});
}

/** Device and app names of one plan, for `issueText` and `exceptionText`. */
export function planNames(t: DevicesT, plan: DeployPlan): IssueNames {
	return {
		app: plan.app?.name ?? t("devices:deploy.thisApp", "this app"),
		device: (deviceId) =>
			plan.targets.find((target) => target.deviceId === deviceId)?.name ??
			t("devices:deploy.aDevice", "a device"),
	};
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
}

const EXCEPTION_COPY = {
	left_out_refuse: ({ t, p, event }) => ({
		differs: t("devices:deploy.exception.leftOut", "{{event}} · left out", {
			event,
		}),
		why: t("devices:deploy.exception.refuseWhy", "Can't run here: {{reason}}", {
			reason: p.reason,
		}),
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
): ExceptionText {
	const p = exception.params ?? {};
	const device = facts?.devices[exception.deviceId];
	const usedBy = device?.portsInUse?.find(
		(row) => row.port === p.from,
	)?.serviceId;
	return EXCEPTION_COPY[exception.code]({
		t,
		p,
		device: planNames(t, plan).device(exception.deviceId),
		event: typeof p.event === "string" ? eventName(plan, p.event) : "",
		...(usedBy ? { usedBy } : {}),
		...(device?.isolation === "none" ? { cantSandbox: true } : {}),
	});
}
