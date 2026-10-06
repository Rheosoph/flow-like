import type { ArtifactUsage } from "../../../../lib/device-management/agent-reads";
import type {
	ArtifactTransferStatus,
	PendingArtifactTransfer,
	PreparedProjectArtifact,
	ProjectArtifactAssets,
} from "../../../../lib/device-management/artifacts";
import {
	botTokenKey,
	isBotTokenKey,
} from "../../../../lib/device-management/bot-config";
import {
	type DeploymentCatalog,
	type DeploymentEvent,
	type DeploymentVariable,
	type InstalledProject,
	type PlacementConfiguration,
	createDeploymentPlan,
	eventEligibility,
	missingFeature,
} from "../../../../lib/device-management/deployment";
import {
	type DeployDraft,
	type DeployPhase,
	type DeployPlan,
	type DeployTargetDraft,
	type PlanApp,
	type PlanTarget,
	type PlanTargetService,
	planPhases,
	wirePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	AgentFeature,
	AgentFeatures,
	CopyParams,
} from "../../../../lib/device-management/model/types";
import { exportTypes } from "../../../../lib/device-management/online-metadata";
import type { DevicesT } from "../primitives/area-context";
import type { DeployPrepared } from "./step-props";

/* The shipping side of a deploy as a model (APP §3.11–§3.14): strategy and phases, what a device holds, what Review shows. The run and the steps read the same answers. */

export type UpdateStrategy = "safe" | "quick";

/** Why a safe update isn't used, in the order the device checks (`canCheckDeploymentStartup`). */
export type SafeUpdateBlocker =
	| "chosen_quick"
	| "not_loaded"
	| "stopped"
	| "source_changed"
	| "agent_unsupported"
	| "no_cloud_access"
	| "no_events"
	| "event_unchecked";

export interface StrategyReason {
	code: SafeUpdateBlocker;
	params?: CopyParams;
}

export type UpdateEvent = Pick<
	DeploymentEvent,
	"id" | "name" | "hosted" | "rollout_supported"
>;

export interface UpdateFacts {
	/** How the app runs after the update. */
	source: "offline" | "online";
	/** The service as the device reports it; undefined while it can't be read (locked). */
	existing?: Pick<
		PlacementConfiguration,
		"desired_state" | "rollout_sources"
	> & {
		config: Pick<PlacementConfiguration["config"], "source" | "resource_grant">;
	};
	/** The events the service serves after the update. */
	events: readonly UpdateEvent[];
}

export interface UpdateStrategyInfo {
	strategy: UpdateStrategy;
	/** False while the device's facts aren't readable: the strategy is decided when they are. */
	known: boolean;
	/** Why it is a quick update; empty for a safe one. */
	reasons: StrategyReason[];
}

export interface SafeUpdateTimings {
	/** Seconds the new version must stay healthy. */
	stabilizeSeconds: number;
	/** Seconds the new version has to become healthy. */
	deadlineSeconds: number;
}

export const SAFE_UPDATE_TIMINGS: SafeUpdateTimings = {
	stabilizeSeconds: 10,
	deadlineSeconds: 120,
};
export const STABILIZE_RANGE = { min: 2, max: 60 } as const;
export const DEADLINE_RANGE = { min: 10, max: 600 } as const;

const within = (value: number, range: { min: number; max: number }) =>
	Number.isInteger(value) && value >= range.min && value <= range.max;

export function timingIssues(
	timings: SafeUpdateTimings,
): ("stabilize" | "deadline")[] {
	return [
		...(within(timings.stabilizeSeconds, STABILIZE_RANGE)
			? []
			: (["stabilize"] as const)),
		...(within(timings.deadlineSeconds, DEADLINE_RANGE)
			? []
			: (["deadline"] as const)),
	];
}

function uncheckedEvents(events: readonly UpdateEvent[]): StrategyReason[] {
	const unchecked = events.filter(
		(event) => !event.hosted && event.rollout_supported !== true,
	);
	const [first] = unchecked;
	if (!first) return [];
	return [
		{
			code: "event_unchecked",
			params: { event: first.name || first.id, count: unchecked.length },
		},
	];
}

function serviceBlockers(
	facts: UpdateFacts & { existing: NonNullable<UpdateFacts["existing"]> },
): StrategyReason[] {
	const { existing, source } = facts;
	const supported = (existing.rollout_sources ?? ["offline"]).includes(source);
	const flagged: [SafeUpdateBlocker, boolean][] = [
		["stopped", existing.desired_state !== "running"],
		["source_changed", existing.config.source !== source],
		["agent_unsupported", !supported],
		["no_cloud_access", source === "online" && !existing.config.resource_grant],
		["no_events", facts.events.length === 0],
	];
	return flagged.filter(([, failing]) => failing).map(([code]) => ({ code }));
}

/** Everything that keeps the device from checking the new version before switching; empty = a safe update is available. */
export function safeUpdateBlockers(facts: UpdateFacts): StrategyReason[] {
	const { existing } = facts;
	if (!existing) return [{ code: "not_loaded" }];
	return [
		...serviceBlockers({ ...facts, existing }),
		...uncheckedEvents(facts.events),
	];
}

/** `auto` = safe when the device can check the new version, else quick with the reasons. */
export function updateStrategy(
	chosen: DeployDraft["strategy"],
	facts: UpdateFacts,
): UpdateStrategyInfo {
	const blockers = safeUpdateBlockers(facts);
	const known = facts.existing !== undefined;
	if (chosen === "quick")
		return { strategy: "quick", known, reasons: [{ code: "chosen_quick" }] };
	return blockers.length
		? { strategy: "quick", known, reasons: blockers }
		: { strategy: "safe", known, reasons: [] };
}

const REASON_COPY: Record<
	SafeUpdateBlocker,
	(t: DevicesT, params: CopyParams) => string
> = {
	chosen_quick: (t) =>
		t("devices:deployShip.strategy.chosenQuick", "you chose a quick update"),
	not_loaded: (t) =>
		t(
			"devices:deployShip.strategy.notLoaded",
			"the service's state loads when the device is unlocked",
		),
	stopped: (t) =>
		t("devices:deployShip.strategy.stopped", "the service is stopped"),
	source_changed: (t) =>
		t(
			"devices:deployShip.strategy.sourceChanged",
			"the service runs the app another way",
		),
	agent_unsupported: (t) =>
		t(
			"devices:deployShip.strategy.agentUnsupported",
			"the device agent can't check this kind of app before switching",
		),
	no_cloud_access: (t) =>
		t(
			"devices:deployShip.strategy.noCloudAccess",
			"the service has no cloud access",
		),
	no_events: (t) =>
		t("devices:deployShip.strategy.noEvents", "no event is left to check"),
	event_unchecked: (t, params) =>
		t(
			"devices:deployShip.strategy.eventUnchecked",
			"{{event}} can't be checked",
			{ event: params.event },
		),
};

/** The reason as a clause ("Quick update: Nightly CRM sync can't be checked"). */
export function strategyReasonText(
	t: DevicesT,
	reason: StrategyReason,
): string {
	return REASON_COPY[reason.code](t, reason.params ?? {});
}

/** What an update sends: nothing (settings or events only), a re-pin (online) or a new copy (offline). */
export type UpdateKind = "settings" | "repin" | "reupload";

export function updateKind(plan: DeployPlan): UpdateKind {
	if (plan.draft.version === "keep") return "settings";
	return plan.mode === "offline" ? "reupload" : "repin";
}

/** A target that keeps its version neither prepares nor uploads anything. */
export function uploadsNothing(
	plan: DeployPlan,
	service: Pick<PlanTargetService, "kind">,
): boolean {
	return service.kind !== "new" && plan.draft.version === "keep";
}

/** The version-only update of several services (Update everywhere). */
export function isFastPath(draft: DeployDraft): boolean {
	return (
		draft.entry === "update" &&
		draft.keepEvents === true &&
		draft.version === "newest"
	);
}

export interface StartChoice {
	kind: PlanTargetService["kind"];
	strategy: UpdateStrategy;
	/** The service runs before the update. */
	wasRunning: boolean;
	/** The draft's "Start after deploy". */
	start: boolean;
	/** "Start it with the new version" for a service that is stopped now. */
	startStopped: boolean;
}

/** Whether the service runs once the target is done. */
export function startsAfter(choice: StartChoice): boolean {
	if (choice.kind === "new") return choice.start;
	if (choice.strategy === "safe") return true;
	return choice.wasRunning ? choice.start : choice.startStopped;
}

/** `stop` is the step that applies the new settings (it stops a running service); `start` only when it runs afterwards. */
function quickPhases(phases: DeployPhase[], choice: StartChoice) {
	return startsAfter(choice)
		? phases
		: phases.filter((phase) => phase !== "start");
}

/** The phases of one service on one device, in the device's order. */
export function targetPhases(
	plan: DeployPlan,
	service: PlanTargetService,
	options: { secrets: boolean } & Omit<StartChoice, "kind" | "start">,
): DeployPhase[] {
	const phases = planPhases(plan, service, {
		secrets: options.secrets,
		safe: options.strategy === "safe",
	});
	if (service.kind === "new" || options.strategy === "safe") return phases;
	return quickPhases(phases, {
		...options,
		kind: service.kind,
		start: plan.draft.start,
	});
}

const draftTarget = (
	plan: DeployPlan,
	deviceId: string,
): DeployTargetDraft | undefined =>
	plan.draft.targets.find((target) => target.deviceId === deviceId);

function secretVariables(plan: DeployPlan, service: PlanTargetService) {
	const ids = new Set<string>();
	for (const eventId of service.events)
		for (const variable of plan.app?.variables?.[eventId] ?? [])
			if (variable.secret) ids.add(variable.id);
	return [...ids];
}

/** Whether a secret has a value for this device: its own, else the shared one when secrets are the same everywhere. */
function hasSecretValue(plan: DeployPlan, deviceId: string, id: string) {
	const { draft } = plan;
	const own = draftTarget(plan, deviceId)?.over.secrets?.[id];
	const shared = draft.secretsMode === "same" ? draft.secrets[id] : undefined;
	return (own ?? shared ?? "") !== "";
}

/** A bot's token is sent where the service gets the bot, and where it is set anew. */
function botTokensToSave(
	plan: DeployPlan,
	deviceId: string,
	service: PlanTargetService,
): number {
	return service.events.filter((eventId) => {
		const key = botTokenKey(eventId);
		const sent =
			service.addedBots.includes(eventId) || plan.draft.edited.includes(key);
		return sent && hasSecretValue(plan, deviceId, key);
	}).length;
}

/** Secrets the device has to save for this service: typed secret values, the token of each bot it adds, plus a new access token. */
export function secretCount(
	plan: DeployPlan,
	target: PlanTarget,
	service: PlanTargetService,
): number {
	const { draft } = plan;
	const typed = secretVariables(plan, service).filter(
		(id) =>
			!isBotTokenKey(id) &&
			(service.kind === "new" || draft.edited.includes(id)) &&
			hasSecretValue(plan, target.deviceId, id),
	).length;
	const hosted = plan.services.find(
		(value) => value.key === service.key,
	)?.hosted;
	const token =
		hosted && !["keep", "none"].includes(draft.endpoint.token) ? 1 : 0;
	return typed + botTokensToSave(plan, target.deviceId, service) + token;
}

/* What a device may be sent (R2 §1.1, §1.9, §1.14). */

const eventsOf = (
	app: Pick<PlanApp, "events"> | null,
	ids: Iterable<string>,
) => {
	const wanted = new Set(ids);
	return (app?.events ?? []).filter((event) => wanted.has(event.id));
};

/** Every event a plan names: the ones it sends and the ones its services keep. */
export function planEventIds(
	plan: Pick<DeployPlan, "services" | "targets">,
): string[] {
	return [
		...new Set([
			...plan.services.flatMap((service) => service.events),
			...plan.targets.flatMap((target) =>
				target.services.flatMap((service) => service.events),
			),
		]),
	];
}

/** The `types` the hub's export is asked for, for these events of the app; none for a hub without `event_types`. */
export function exportTypesOf(
	app: Pick<PlanApp, "events"> | null,
	eventIds: Iterable<string>,
	hubTypes: readonly string[] | undefined,
): string[] {
	return exportTypes(eventsOf(app, eventIds), hubTypes);
}

/** The export's `types` for a plan: those of the events it sends and of those its services keep. */
export function planExportTypes(
	plan: Pick<DeployPlan, "app" | "services" | "targets">,
	hubTypes: readonly string[] | undefined,
): string[] {
	return exportTypesOf(plan.app, planEventIds(plan), hubTypes);
}

export interface AgentGap {
	eventId: string;
	/** `unknown`: the agent reports no flags, and the event needs one. */
	feature: AgentFeature | "unknown";
}

/**
 * The first event of a service that the device's agent can't run: an agent
 * without a flag accepts the service and then fails it on every start, so
 * such a service is never sent (the hard gate). Null when every event runs.
 */
export function agentGap(
	app: Pick<PlanApp, "events"> | null,
	service: Pick<PlanTargetService, "events">,
	features: AgentFeatures | undefined,
): AgentGap | null {
	for (const event of eventsOf(app, service.events)) {
		const feature = missingFeature(event, features);
		if (feature) return { eventId: event.id, feature };
	}
	return null;
}

/** Whether any event of the service needs an agent flag: only then is the agent asked again before sending. */
export function needsAgentFlags(
	app: Pick<PlanApp, "events"> | null,
	service: Pick<PlanTargetService, "events">,
): boolean {
	return agentGap(app, service, {}) !== null;
}

/**
 * The first event of a service that the catalogue doesn't carry as one a
 * device can run. A service is wired from the catalogue, so without this an
 * event would be dropped from the service without a word.
 */
export function catalogGap(
	catalog: DeploymentCatalog,
	service: Pick<PlanTargetService, "events">,
): string | null {
	return (
		service.events.find(
			(eventId) =>
				!catalog.events.some((event) => event.id === eventId && event.eligible),
		) ?? null
	);
}

/** A staged, validating or activating update blocks a new one for that service. */
export function blocksNewUpdate(state: string | null | undefined): boolean {
	return ["staged", "validating", "activating", "rolling_back"].includes(
		state ?? "",
	);
}

/** Drift is per service: one service of an app can be behind while another on the same device is current. */
export function isBehind(
	service: { appVersion: { hash?: string } | null },
	newestHash: string | null | undefined,
): boolean {
	const running = service.appVersion?.hash;
	return Boolean(running && newestHash && running !== newestHash);
}

/* What a device holds once a version is there. */

/** A failure the run words itself (`code`), with the underlying sentence for Details. */
export class DeployRunFailure extends Error {
	constructor(
		readonly code: string,
		detail?: string,
	) {
		super(detail ?? code);
		this.name = "DeployRunFailure";
	}
}

export const deployTarget = (deviceId: string, serviceId: string) =>
	`${deviceId}/${serviceId}`;

function assetsOf(artifact: PreparedProjectArtifact): ProjectArtifactAssets {
	const manifest = JSON.parse(
		new TextDecoder().decode(artifact.manifest),
	) as Partial<ProjectArtifactAssets>;
	return {
		bit_pins: manifest.bit_pins ?? [],
		package_pins: manifest.package_pins ?? [],
	};
}

/** The project as the device holds it once the bundle is committed. */
export function installedProject(
	bundle: DeployPrepared,
	projectPath: string,
): InstalledProject {
	const { descriptor } = bundle.artifact;
	const source = descriptor.source ?? "offline";
	if (source === "online" && !bundle.approved)
		throw new DeployRunFailure("not_prepared");
	return {
		project_id: descriptor.project_id,
		project_path: projectPath,
		revision: descriptor.manifest_sha256,
		source,
		assets: assetsOf(bundle.artifact),
		...(bundle.approved
			? {
					online_metadata_sha256: bundle.approved.sha256,
					online_catalog: bundle.approved.catalog,
				}
			: {}),
	};
}

/** The version a service already runs, for an update that keeps it. */
export function installedFromExisting(
	existing: PlacementConfiguration,
): InstalledProject {
	const { config } = existing;
	return {
		project_id: existing.project_id,
		project_path: config.project_path,
		revision: config.revision,
		source: config.source,
		...(config.online_metadata_sha256
			? { online_metadata_sha256: config.online_metadata_sha256 }
			: {}),
	};
}

type FlowPin = [number, number, number];

/**
 * An event that follows Latest has no flow pin of its own: it ships at the
 * version the preparation resolved, and a service that keeps its version
 * keeps the pin it has. Without either it can't be sent.
 */
function keptEvent(
	event: NonNullable<DeployPlan["app"]>["events"][number],
	pins: PlacementConfiguration["config"]["events"][number] | undefined,
	resolved: FlowPin | undefined,
): DeploymentEvent[] {
	const facts = eventEligibility(event);
	const eventVersion = pins?.event_version ?? facts.eventVersion;
	const boardVersion = facts.followsLatest
		? (resolved ?? pins?.board_version ?? null)
		: (pins?.board_version ?? facts.boardVersion);
	if (!eventVersion || !boardVersion) return [];
	return [
		{
			id: event.id,
			name: event.name,
			event_type: event.event_type,
			event_version: eventVersion,
			board_version: boardVersion,
			hosted: facts.hosted,
			rollout_supported: facts.rolloutSupported,
			eligible: true,
		},
	];
}

/**
 * The catalogue without an approved bundle: the app's events with their
 * published pins, or the pins a service keeps. `latest` holds the flow version
 * each event that follows Latest was resolved to when the copy was prepared.
 */
export function planCatalog(
	plan: DeployPlan,
	existing?: PlacementConfiguration,
	latest?: DeployPrepared["latest"],
): DeploymentCatalog {
	const pinned = new Map(
		(existing?.config.events ?? []).map((event) => [event.event_id, event]),
	);
	const app = plan.app;
	return {
		events: (app?.events ?? []).flatMap((event) =>
			keptEvent(event, pinned.get(event.id), latest?.[event.id]),
		),
		variables: Object.fromEntries(
			Object.entries(app?.variables ?? {}).map(([id, list]) => [id, [...list]]),
		),
	};
}

/* Review (APP §3.12): what each target would get, computed with the wiring the rollout uses. */

const PLACEHOLDER_ID = "00000000-0000-0000-0000-000000000000";
const PLACEHOLDER_TOKEN = "x".repeat(43);

export type ReviewUnread = "locked" | "loading" | "failed";
export type WireProblem =
	| "too_large"
	| "no_changes"
	| "needs_newest"
	| "invalid"
	| "not_prepared";

export interface ReviewTarget {
	key: string;
	target: PlanTarget;
	service: PlanTargetService;
	hosted: boolean;
	existing?: PlacementConfiguration;
	/** Why the service this target updates isn't readable yet. */
	unread?: ReviewUnread;
	strategy?: UpdateStrategyInfo;
	config?: Record<string, unknown>;
	bytes?: number;
	problem?: WireProblem;
}

export interface ExistingRead {
	data?: PlacementConfiguration;
	unread?: ReviewUnread;
}

type Bundle = DeployPrepared | null | undefined;

const sourceOf = (plan: DeployPlan) =>
	plan.mode === "offline" ? ("offline" as const) : ("online" as const);

/** New online services always get an approval; offline copies only for hosted models. */
export const wantsAccess = (plan: DeployPlan) =>
	plan.mode === "online" || plan.draft.approval.models.length > 0;

/** The bundle as the device would hold it; the path only matters for the size estimate. */
function bundleInstalled(bundle: DeployPrepared): InstalledProject | undefined {
	const { descriptor } = bundle.artifact;
	const path = [
		"/var/lib/flow-like/projects",
		descriptor.project_id,
		"revisions",
		descriptor.manifest_sha256,
	].join("/");
	try {
		return installedProject(bundle, path);
	} catch {
		return undefined;
	}
}

function projectedInstalled(
	plan: DeployPlan,
	bundle: Bundle,
	existing: PlacementConfiguration | undefined,
): InstalledProject | undefined {
	const keeps = plan.draft.version === "keep";
	if (existing && keeps) return installedFromExisting(existing);
	return bundle ? bundleInstalled(bundle) : undefined;
}

function reviewToken(plan: DeployPlan, item: ReviewTarget): string {
	const { endpoint } = plan.draft;
	if (!item.hosted || endpoint.token === "none") return "";
	if (endpoint.token === "keep")
		return item.existing?.config.hosting ? "" : PLACEHOLDER_TOKEN;
	const own = draftTarget(plan, item.target.deviceId)?.over.token;
	return own || endpoint.tokenValue || PLACEHOLDER_TOKEN;
}

function reviewGrant(plan: DeployPlan, item: ReviewTarget) {
	if (item.service.kind !== "new" || !wantsAccess(plan)) return undefined;
	return {
		grant_id: PLACEHOLDER_ID,
		authz_version: 1,
		...(plan.draft.spending
			? { billing_grant_id: PLACEHOLDER_ID, billing_authz_version: 1 }
			: {}),
	};
}

/** The lib words its refusals; Review only needs which kind it is (the sentence itself is the lib's English). */
function wireProblem(error: unknown): Pick<ReviewTarget, "problem" | "bytes"> {
	const message = error instanceof Error ? error.message : "";
	const size = /needs (\d+) bytes/u.exec(message);
	if (size) return { problem: "too_large", bytes: Number(size[1]) };
	if (message.includes("no configuration changes"))
		return { problem: "no_changes" };
	// An online service from before versions carried approved definitions can't keep its version.
	if (message.includes("controller-approved executable metadata"))
		return { problem: "needs_newest" };
	return { problem: "invalid" };
}

const byteLength = (value: unknown) =>
	new TextEncoder().encode(JSON.stringify(value)).length;

function wireForReview(
	plan: DeployPlan,
	item: ReviewTarget,
	installed: InstalledProject | undefined,
	catalog: DeploymentCatalog,
): Pick<ReviewTarget, "config" | "bytes" | "problem"> {
	if (!installed) return { problem: "not_prepared" };
	try {
		const deployment = createDeploymentPlan(
			wirePlan(
				plan,
				{ deviceId: item.target.deviceId, serviceKey: item.service.key },
				{
					installed,
					existing: item.existing,
					events: catalog.events,
					variables: catalog.variables,
					previousVariables: [],
					serviceToken: reviewToken(plan, item),
					resourceGrant: reviewGrant(plan, item),
					canManageCertificates: true,
				},
			),
		);
		const bytes = Math.max(
			...deployment.steps.map((step) => byteLength(step.command)),
		);
		return { config: deployment.config, bytes };
	} catch (error) {
		return wireProblem(error);
	}
}

/** The target as far as the plan and the service's current settings say; an update whose service isn't read yet says why. */
function baseTarget(
	plan: DeployPlan,
	target: PlanTarget,
	service: PlanTargetService,
	read: ExistingRead | undefined,
): ReviewTarget {
	const planned = plan.services.find((value) => value.key === service.key);
	const base = {
		key: deployTarget(target.deviceId, service.serviceId),
		target,
		service,
		hosted: planned?.hosted === true,
	};
	if (service.kind === "new") return base;
	const existing = read?.data;
	return existing
		? { ...base, existing }
		: { ...base, unread: read?.unread ?? "loading" };
}

function strategyOf(
	plan: DeployPlan,
	base: ReviewTarget,
	catalog: DeploymentCatalog,
): UpdateStrategyInfo | undefined {
	if (base.service.kind === "new") return undefined;
	return updateStrategy(plan.draft.strategy, {
		source: sourceOf(plan),
		existing: base.existing,
		events: catalog.events.filter((event) =>
			base.service.events.includes(event.id),
		),
	});
}

function reviewTarget(
	plan: DeployPlan,
	bundle: Bundle,
	base: ReviewTarget,
): ReviewTarget {
	const { existing } = base;
	// A service that keeps its version keeps its pins; a new copy carries the ones it was prepared with.
	const resolved = plan.draft.version === "keep" ? undefined : bundle?.latest;
	const catalog =
		bundle?.approved?.catalog ?? planCatalog(plan, existing, resolved);
	const strategy = strategyOf(plan, base, catalog);
	if (base.unread) return { ...base, strategy };
	const installed = projectedInstalled(plan, bundle, existing);
	return {
		...base,
		strategy,
		...wireForReview(plan, base, installed, catalog),
	};
}

/** One row per served service on each device, with what the device would be sent. */
export function reviewTargets(
	plan: DeployPlan,
	bundle: Bundle,
	reads: ReadonlyMap<string, ExistingRead>,
): ReviewTarget[] {
	return plan.targets.flatMap((target) =>
		target.services
			.filter((service) => service.events.length > 0)
			.map((service) => {
				const key = deployTarget(target.deviceId, service.serviceId);
				const base = baseTarget(plan, target, service, reads.get(key));
				return reviewTarget(plan, bundle, base);
			}),
	);
}

/* Copy & upload (APP §3.11): what each device holds of this version and what is left to send. */

export interface CopyRefusedEvent {
	id: string;
	name: string;
	/** The device's own sentence, when it gave one. */
	reason?: string;
	/** `latest_copy`: the event follows Latest and the copy holds no published version of its flow (a copy prepared elsewhere). */
	cause?: "latest_copy";
}

/** What the device said about the copy it now holds, for the events this plan serves there. */
export interface CopyEventCheck {
	/** Names of the events it can run. */
	runs: string[];
	refused: CopyRefusedEvent[];
	/** Event id → the settings the device found for an event it can run. */
	variables: Record<string, DeploymentVariable[]>;
}

export interface CopyUploadEntry {
	phase: "sending" | "busy" | "done" | "failed";
	done: number;
	total: number;
	/** Why it failed, as a sentence. */
	reason?: string;
	/** Unix seconds it finished. */
	at?: number;
	check?: CopyEventCheck;
}

export type CopyUploadEntries = Readonly<Record<string, CopyUploadEntry>>;

export interface DeviceCopyFacts {
	/** Undefined when the agent can't report its storage (older agent) or the read failed. */
	usage?: ArtifactUsage;
	pending?: PendingArtifactTransfer;
	transfer?: ArtifactTransferStatus | null;
}

export interface CopyRow {
	target: PlanTarget;
	key: string;
	/** Nothing is sent: the service keeps its version, or the device already has this one. */
	skip?: "keep" | "has_version";
	have: number;
	total: number;
	sendFiles: number;
	sendBytes: number;
	paused?: { transferId: string; expiresAt: number; confirmed: boolean };
	entry?: CopyUploadEntry;
	usage?: ArtifactUsage;
}

export interface CopyRowInput {
	plan: DeployPlan;
	bundle: DeployPrepared;
	/** Sizes of the bundle's files in upload order. */
	sizes: readonly number[];
	facts: ReadonlyMap<string, DeviceCopyFacts>;
	entries: CopyUploadEntries;
}

export const copyUploadKey = (deviceId: string, bundle: DeployPrepared) =>
	`${deviceId}/${bundle.artifact.descriptor.manifest_sha256}`;

interface ManifestFile {
	size: number;
}

interface ManifestFiles {
	files?: ManifestFile[];
}

const fileSize = (file: ManifestFile): number => file.size;

/** The sizes of the bundle's files, in the order they are sent. */
export const manifestFileSizes = (bundle: DeployPrepared): number[] => {
	const text = new TextDecoder().decode(bundle.artifact.manifest);
	try {
		const manifest: ManifestFiles = JSON.parse(text);
		return (manifest.files ?? []).map(fileSize);
	} catch (_error) {
		return [];
	}
};

const copySkip = (
	input: CopyRowInput,
	target: PlanTarget,
	facts: DeviceCopyFacts | undefined,
): CopyRow["skip"] => {
	const keeps = target.services.every((service) =>
		uploadsNothing(input.plan, service),
	);
	if (keeps) return "keep";
	const sha = input.bundle.artifact.descriptor.manifest_sha256;
	const held = facts?.usage?.revisions.some((row) => row.revision === sha);
	const committed = facts?.transfer?.state === "committed";
	return held || committed ? "has_version" : undefined;
};

const copyPaused = (
	facts: DeviceCopyFacts | undefined,
	entry: CopyUploadEntry | undefined,
): CopyRow["paused"] => {
	const pending = facts?.pending;
	const settled = ["sending", "busy", "done"].includes(entry?.phase ?? "");
	// `null`: the device no longer holds the upload this browser remembers.
	if (!pending || settled || facts?.transfer === null) return undefined;
	return {
		transferId: pending.transfer_id,
		expiresAt: facts?.transfer?.expires_at ?? pending.expires_at,
		confirmed: pending.confirmed === true,
	};
};

/** Files of this version the device already holds: this window's upload, or one it paused earlier. */
function filesHeld(
	facts: DeviceCopyFacts | undefined,
	entry: CopyUploadEntry | undefined,
): number {
	const transfer = facts?.transfer;
	const received =
		transfer?.state === "receiving" ? (transfer.file_index ?? 0) : 0;
	return Math.max(entry?.done ?? 0, received);
}

export function copyRowOf(input: CopyRowInput, target: PlanTarget): CopyRow {
	const { bundle, sizes } = input;
	const key = copyUploadKey(target.deviceId, bundle);
	const facts = input.facts.get(target.deviceId);
	const entry = input.entries[key];
	const { file_count: total, total_bytes: bytes } = bundle.artifact.descriptor;
	const done = entry?.phase === "done";
	const skip = done ? undefined : copySkip(input, target, facts);
	const settled = done || skip !== undefined;
	const have = settled ? total : filesHeld(facts, entry);
	const rest = sizes.slice(have).reduce((sum, size) => sum + size, 0);
	return {
		target,
		key,
		skip,
		have,
		total,
		sendFiles: Math.max(0, total - have),
		sendBytes: have ? rest : bytes,
		paused: skip ? undefined : copyPaused(facts, entry),
		entry,
		usage: facts?.usage,
	};
}
