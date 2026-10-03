import type { DeploymentRolloutStatus } from "../deployment";
import type { RetainedObservation } from "../inventory";
import type { OfflineQueueStatus } from "../offline-queue";
import type {
	KeySessionSnapshot,
	LiveState,
	LocalVaultSummary,
} from "../workspace/types";
import type {
	AttentionCandidateExt,
	AttentionInputExt,
	AttentionRuleExt,
	LiveDeviceInputExt,
} from "./attention";
import { convergence } from "./convergence";
import { classify } from "./freshness";
import { presence, relationshipOf } from "./presence";
import type {
	AgeState,
	AgentFeature,
	AgentFeatures,
	AttentionAction,
	AttentionInput,
	AttentionItem,
	AttentionKey,
	AttentionSubject,
	CopyParams,
	CopyRef,
	DeviceRoute,
	DeviceRow,
	DeviceTab,
	DeviceViewModel,
	Freshness,
	FreshnessReason,
	HealthLevel,
	InspectionPlus,
	PlacementStatusPlus,
	Presence,
	Relationship,
	ServiceRoute,
	ServiceTab,
	ServiceView,
	Severity,
} from "./types";

export const MINUTE_S = 60;
export const HOUR_S = 3_600;
export const DAY_S = 86_400;

/** `ActivityItem` times are unix milliseconds; every model time is unix seconds. */
export const activityS = (ms: number) => Math.floor(ms / 1_000);

export interface ServicesUnavailable {
	state: AgeState;
	reason?: CopyRef<FreshnessReason>;
}

/** Everything the rules and views derive once per input (cached per input object). */
export interface DeviceFacts {
	id: string;
	row: DeviceRow;
	name: string;
	active: boolean;
	presence: Presence;
	relationship: Relationship;
	keys: KeySessionSnapshot;
	vault?: LocalVaultSummary;
	live: LiveState;
	liveInput?: LiveDeviceInputExt;
	/** A session is open (live or renewing): live facts are current. */
	liveOpen: boolean;
	inspection?: InspectionPlus;
	inspectionSource?: Freshness;
	services: ServiceView[] | ServicesUnavailable;
	/** The agent's flags on the plane the services come from; absent = unknown, never "too old". */
	features?: AgentFeatures;
	/** The installed release; `sequence` when the agent reports it (BG8). */
	agent?: { version: string; sequence?: number; source: Freshness };
}

export interface FleetFacts {
	devices: readonly DeviceFacts[];
	byId: ReadonlyMap<string, DeviceFacts>;
}

const factsCache = new WeakMap<AttentionInput, FleetFacts>();

export function fleetFacts(input: AttentionInputExt): FleetFacts {
	const cached = factsCache.get(input);
	if (cached) return cached;
	const devices = input.devices.map((row) => deviceFacts(input, row));
	const facts = {
		devices,
		byId: new Map(devices.map((device) => [device.id, device])),
	};
	factsCache.set(input, facts);
	return facts;
}

/**
 * Whether "not in the hub list" means anything yet. Before the list has loaded,
 * or while it can't be read, `devices` is empty and says nothing about a device.
 */
export function deviceListKnown(input: AttentionInputExt): boolean {
	return input.devicesLoaded ?? input.devices.length > 0;
}

export function deviceName(row: Pick<DeviceRow, "name" | "display_name">) {
	return row.display_name || row.name;
}

/** Name for a device that may be missing from the hub list (local vaults, requests). */
export function deviceLabel(input: AttentionInputExt, deviceId: string) {
	const row = fleetFacts(input).byId.get(deviceId);
	if (row) return row.name;
	const request = input.accessRequests?.find(
		(entry) => entry.deviceId === deviceId,
	);
	return request?.deviceName ?? deviceId.slice(0, 8);
}

function deviceFacts(input: AttentionInputExt, row: DeviceRow): DeviceFacts {
	const id = row.device_id;
	const relationship = relationshipOf(row, input.me);
	const vault = input.local.vaults.find((entry) => entry.deviceId === id);
	const keys =
		input.keys.find((entry) => entry.deviceId === id) ??
		defaultKeys(id, vault, relationship);
	const liveInput = input.live[id];
	const live: LiveState = liveInput?.state ?? { kind: "idle" };
	const liveOpen = live.kind === "live" || live.kind === "renewing";
	const inspection = liveInput?.inspection?.value;
	const inspectionSource = liveInput?.inspection
		? classify("live_inspection", {
				now: input.now,
				at: liveInput.inspection.readAt,
				loaded: true,
				sessionOpen: liveOpen,
			})
		: undefined;
	const facts: DeviceFacts = {
		id,
		row,
		name: deviceName(row),
		active: row.status === "active",
		presence: presence(row, input.now),
		relationship,
		keys,
		vault,
		live,
		liveInput,
		liveOpen,
		inspection,
		inspectionSource,
		services: [],
	};
	Object.assign(facts, readServices(input, facts));
	facts.agent = agentOf(input, facts);
	return facts;
}

function defaultKeys(
	deviceId: string,
	vault: LocalVaultSummary | undefined,
	relationship: Relationship,
): KeySessionSnapshot {
	return {
		deviceId,
		state: vault ? "locked" : "none",
		role: vault?.role ?? (relationship === "shared" ? "shared" : "owner"),
		grantId: vault?.grantId ?? "",
		canSign: false,
		keepUnlocked: false,
		restoredNeedsFreshEndpoint: vault?.requiresFreshEndpoint ?? false,
	};
}

/**
 * The agent's own `version` (and `agent_version`) is its crate version, the
 * constant 0.1.0: only the installed release says what runs (M-BACK C3, BG8).
 */
function agentOf(
	input: AttentionInputExt,
	facts: DeviceFacts,
): DeviceFacts["agent"] {
	const release = facts.inspection?.agent;
	if (release?.release_version && facts.inspectionSource)
		return {
			version: release.release_version,
			...(release.release_sequence == null
				? {}
				: { sequence: release.release_sequence }),
			source: facts.inspectionSource,
		};
	const last = input.agentLastRead?.[facts.id];
	if (!last) return undefined;
	return {
		version: last.version,
		...(last.sequence == null ? {} : { sequence: last.sequence }),
		source: classify("saved_inventory", {
			now: input.now,
			at: last.at,
			loaded: true,
		}),
	};
}

interface PlacementSource {
	placements: PlacementStatusPlus[];
	freshness: Freshness;
	at: number;
	/** The agent's flags on this plane; absent = this plane does not say. */
	features?: AgentFeatures;
}

/** A snapshot carries the flags only for a device-scope reader, on the observation that holds the device facts. */
const snapshotFeatures = (
	observations: readonly { features?: AgentFeatures }[],
): AgentFeatures | undefined =>
	observations.find((observation) => observation.features)?.features;

/** Newest row per placement id across observations (`observed_at` is milliseconds). */
function latestPlacements(
	observations: readonly RetainedObservation[],
): PlacementStatusPlus[] {
	const rows = new Map<string, { row: PlacementStatusPlus; at: number }>();
	for (const observation of observations)
		for (const row of observation.placements) {
			const at = observation.observed_at ?? 0;
			const known = rows.get(row.id);
			if (!known || known.at < at) rows.set(row.id, { row, at });
		}
	return [...rows.values()].map((entry) => entry.row);
}

/** IA §2.2: a failed read keeps the last good rows and says so on their stamp. */
const keptError = (freshness: Freshness) =>
	freshness.error ? { error: freshness.error } : {};

function placementSources(
	input: AttentionInputExt,
	facts: DeviceFacts,
): PlacementSource[] {
	const sources: PlacementSource[] = [];
	const fleet = input.fleet[facts.id];
	if (fleet?.status) {
		const features = snapshotFeatures(fleet.status.observations);
		sources.push({
			placements: latestPlacements(fleet.status.observations),
			at: fleet.status.observedAt,
			freshness: classify("fleet_status", {
				now: input.now,
				at: fleet.status.observedAt,
				loaded: true,
				...keptError(fleet.freshness.status),
			}),
			...(features ? { features } : {}),
		});
	}
	if (fleet?.saved)
		sources.push({
			placements: latestPlacements(fleet.saved.observations),
			at: fleet.saved.observedAt,
			freshness: classify("saved_inventory", {
				now: input.now,
				at: fleet.saved.observedAt,
				loaded: true,
				...keptError(fleet.freshness.saved),
			}),
		});
	const inspection = facts.liveInput?.inspection;
	if (inspection && facts.inspectionSource)
		sources.push({
			placements: inspection.value.placements,
			at: inspection.readAt,
			freshness: facts.inspectionSource,
			features: inspection.value.features,
		});
	return sources;
}

/** Live while a session is open, otherwise the most recent of snapshot, saved inventory and the last live read. */
function pickSource(
	sources: PlacementSource[],
	liveOpen: boolean,
): PlacementSource | undefined {
	if (liveOpen) {
		const live = sources.find((source) => source.freshness.src === "live");
		if (live) return live;
	}
	return sources.reduce<PlacementSource | undefined>(
		(best, source) => (!best || source.at > best.at ? source : best),
		undefined,
	);
}

/** The device's service rows and the agent flags of the plane they come from. */
function readServices(
	input: AttentionInputExt,
	facts: DeviceFacts,
): Pick<DeviceFacts, "services" | "features"> {
	const unavailable = servicesUnavailable(input, facts);
	if (unavailable) return { services: unavailable };
	const source = pickSource(placementSources(input, facts), facts.liveOpen);
	if (!source) return { services: { state: "notloaded" } };
	return {
		services: source.placements.map((placement) =>
			serviceView(facts.id, placement, source, facts.liveInput),
		),
		...(source.features ? { features: source.features } : {}),
	};
}

const LOCKED_KEYS = new Set<KeySessionSnapshot["state"]>([
	"locked",
	"unlocking",
	"held_elsewhere",
	"blocked",
]);

/** Keys exist here and an unlock opens them (locked, unlocking, held by another tab, blocked). */
export function keysLocked(keys: Pick<KeySessionSnapshot, "state">) {
	return LOCKED_KEYS.has(keys.state);
}

/** Why this device's rows can't be read, from the hub row and the keys alone. */
const deviceUnavailable = (
	facts: DeviceFacts,
): ServicesUnavailable | undefined => {
	if (!facts.active) return { state: "notloaded" };
	if (facts.presence.kind === "never")
		return { state: "notloaded", reason: { code: "never_reported" } };
	if (facts.keys.state === "none")
		return { state: "notloaded", reason: { code: "no_keys_here" } };
	// Keys for access that ended can't be unlocked any more: never "Locked".
	if (facts.keys.state === "stale")
		return { state: "noaccess", reason: { code: "access_ended" } };
	if (LOCKED_KEYS.has(facts.keys.state))
		return { state: "locked", reason: { code: "unlock_required" } };
	return undefined;
};

const SNAPSHOT_BLOCKED = new Set<AgeState>([
	"noaccess",
	"unsupported",
	"error",
]);

type FleetState = AttentionInputExt["fleet"][string];

/** A snapshot that can't be read; a failed read that still has earlier rows keeps them (IA §2.2). */
const snapshotBlocked = (fleet: FleetState): Freshness | undefined => {
	const status = fleet.freshness.status;
	if (!SNAPSHOT_BLOCKED.has(status.age)) return undefined;
	const keepsRows = status.age === "error" && (fleet.status || fleet.saved);
	return keepsRows ? undefined : status;
};

/** Without a live read, a snapshot that can't be read explains the gap. */
const snapshotUnavailable = (
	input: AttentionInputExt,
	facts: DeviceFacts,
): ServicesUnavailable | undefined => {
	const fleet = input.fleet[facts.id];
	if (facts.liveInput?.inspection || !fleet) return undefined;
	const status = snapshotBlocked(fleet);
	if (!status) return undefined;
	const reason =
		status.reason ?? (status.error ? { code: status.error.code } : undefined);
	return reason ? { state: status.age, reason } : { state: status.age };
};

const servicesUnavailable = (input: AttentionInputExt, facts: DeviceFacts) =>
	deviceUnavailable(facts) ?? snapshotUnavailable(input, facts);

const ACTIVE_ROLLOUT = new Set<DeploymentRolloutStatus["state"]>([
	"staged",
	"validating",
	"activating",
	"rolling_back",
]);

/** The device discards an update that stays staged this long (`staging_timeout`). */
export const STAGED_ROLLOUT_TTL_S = DAY_S;

/**
 * When the device ends this rollout on its own. A staged update has no
 * `deadline_at` yet: it is discarded a day after it was staged.
 */
export function rolloutEndsAt(
	rollout: DeploymentRolloutStatus,
): number | undefined {
	if (rollout.state !== "staged") return rollout.deadline_at ?? undefined;
	return rollout.created_at === undefined
		? undefined
		: rollout.created_at + STAGED_ROLLOUT_TTL_S;
}

const rolloutTime = (rollout: DeploymentRolloutStatus) =>
	rollout.updated_at ?? rollout.created_at ?? 0;

const latestRollout = (rollouts: readonly DeploymentRolloutStatus[]) =>
	rollouts.reduce<DeploymentRolloutStatus | undefined>(
		(latest, rollout) =>
			!latest || rolloutTime(rollout) > rolloutTime(latest) ? rollout : latest,
		undefined,
	);

/** The rollout that describes the placement now: an active one, else the most recently updated. */
export const currentRollout = (
	rollouts: readonly DeploymentRolloutStatus[] | undefined,
	placementId: string,
): DeploymentRolloutStatus | undefined => {
	const own = (rollouts ?? []).filter(
		(rollout) => rollout.placement_id === placementId,
	);
	return (
		own.find((rollout) => ACTIVE_ROLLOUT.has(rollout.state)) ??
		latestRollout(own)
	);
};

const HEAD_RANK: Record<
	NonNullable<OfflineQueueStatus["head"]>["state"],
	number
> = { conflict: 0, blocked: 1, outcome_unknown: 2, attempting: 3, pending: 4 };

function offlineWritesOf(
	placement: PlacementStatusPlus,
	live: LiveDeviceInputExt | undefined,
): ServiceView["offlineWrites"] {
	const queues = live?.offlineQueues?.[placement.id];
	if (queues) {
		const head = queues
			.map((queue) => queue.head)
			.filter((entry) => entry !== null)
			.sort((a, b) => HEAD_RANK[a.state] - HEAD_RANK[b.state])[0];
		const pending = queues.reduce((sum, queue) => sum + queue.pending_count, 0);
		const quarantined = queues.some((queue) => queue.quarantined);
		return head ? { pending, head, quarantined } : { pending, quarantined };
	}
	const summary = placement.offline_writes;
	if (summary)
		return {
			pending: summary.pending_count,
			quarantined: summary.quarantined_scopes > 0,
		};
	return "not_loaded";
}

function diagnosticsOf(
	placement: PlacementStatusPlus,
): ServiceView["diagnostics"] {
	const restarts =
		placement.restarts ??
		placement.replicas?.find((replica) => replica.restarts)?.restarts;
	const lastError =
		placement.last_error ??
		placement.replicas?.find((replica) => replica.last_error)?.last_error;
	const replicaError = placement.replicas?.some(
		(replica) => replica.has_error === true,
	);
	if (
		placement.has_error === undefined &&
		!replicaError &&
		!lastError &&
		!restarts
	)
		return undefined;
	return {
		hasError: placement.has_error ?? (replicaError || !!lastError),
		...(lastError ? { lastError } : {}),
		...(restarts ? { restarts } : {}),
	};
}

type ReportedList<T> = T[] | "not_reported" | "needs_agent" | "not_loaded";

/**
 * A row carries `schedules`, `bots` and `actions` only while a process of the
 * service runs and reported them (a finished one-time schedule also while it
 * is stopped). Without the key, the agent's flags decide what that means: not
 * running or not reported yet, an agent too old to say, or unknown.
 */
function reportedList<T>(
	list: T[] | undefined,
	features: AgentFeatures | undefined,
	flags: readonly AgentFeature[],
): ReportedList<T> {
	if (list) return list;
	if (!features) return "not_loaded";
	return flags.some((flag) => features[flag]) ? "not_reported" : "needs_agent";
}

function rowListsOf(
	placement: PlacementStatusPlus,
	features: AgentFeatures | undefined,
): Pick<
	ServiceView,
	| "schedules"
	| "schedulesTruncated"
	| "bots"
	| "botsTruncated"
	| "actions"
	| "actionsTruncated"
> {
	return {
		schedules: reportedList(placement.schedules, features, [
			"scheduled_events",
		]),
		...(placement.schedules && placement.schedules_truncated
			? { schedulesTruncated: true }
			: {}),
		bots: reportedList(placement.bots, features, [
			"telegram_bots",
			"discord_bots",
		]),
		...(placement.bots && placement.bots_truncated
			? { botsTruncated: true }
			: {}),
		actions: reportedList(placement.actions, features, ["on_demand_events"]),
		...(placement.actions && placement.actions_truncated
			? { actionsTruncated: true }
			: {}),
	};
}

function serviceView(
	deviceId: string,
	placement: PlacementStatusPlus,
	{ freshness, features }: Pick<PlacementSource, "freshness" | "features">,
	live: LiveDeviceInputExt | undefined,
): ServiceView {
	const rollout = currentRollout(live?.rollouts, placement.id);
	const observedRunning = placement.observed_state === "running";
	const requested = placement.desired_replicas ?? 1;
	const view: ServiceView = {
		deviceId,
		serviceId: placement.id,
		projectId: placement.project_id,
		deploymentId: placement.deployment_id,
		desired: placement.desired_state,
		observed: placement.observed_state,
		conv: convergence(placement, rollout),
		settings: {
			applied: placement.applied_revision,
			latest: placement.config_revision,
		},
		instances: {
			requested,
			ready: placement.ready_replicas ?? (observedRunning ? requested : 0),
			running: placement.running_replicas ?? (observedRunning ? requested : 0),
			max: placement.max_replicas ?? requested,
		},
		freshness,
		source: placement.source ?? null,
		events: placement.events ?? null,
		appVersion: placement.revision ? { hash: placement.revision } : null,
		offlineWrites: offlineWritesOf(placement, live),
		...rowListsOf(placement, features),
	};
	const diagnostics = diagnosticsOf(placement);
	if (diagnostics) view.diagnostics = diagnostics;
	if (rollout) view.rollout = rollout;
	return view;
}

/** Service rows for one device; never an empty array for "not loaded". */
export function buildServiceViews(
	deviceId: string,
	input: AttentionInputExt,
): ServiceView[] | ServicesUnavailable {
	return (
		fleetFacts(input).byId.get(deviceId)?.services ?? { state: "notloaded" }
	);
}

/** Facts read from the encrypted snapshot or live are "last known" unless current. */
export function isLastKnown(freshness: Freshness) {
	return freshness.age !== "live" && freshness.age !== "current";
}

export function subjectDevice(subject: AttentionSubject): string | undefined {
	return "deviceId" in subject ? subject.deviceId : undefined;
}

export function itemsForDevice(
	items: readonly AttentionItem[],
	deviceId: string,
): AttentionItem[] {
	return items.filter((item) => subjectDevice(item.subject) === deviceId);
}

/** IA §6.5 rollup. Info is never counted; "unknown" when nothing but the hub row is readable. */
export function rollupHealth(
	items: readonly AttentionItem[],
	device: Pick<DeviceRow, "device_id" | "status">,
	hasData: boolean,
): HealthLevel {
	if (device.status === "revoked") return "revoked";
	const counted = itemsForDevice(items, device.device_id).filter(
		(item) => item.severity !== "info",
	);
	if (counted.some((item) => item.severity === "critical")) return "critical";
	if (counted.length) return "attention";
	return hasData ? "healthy" : "unknown";
}

export function buildDeviceView(
	deviceId: string,
	input: AttentionInputExt,
	items: readonly AttentionItem[],
): DeviceViewModel | undefined {
	const facts = fleetFacts(input).byId.get(deviceId);
	if (!facts) return undefined;
	const attention = itemsForDevice(items, deviceId);
	const view: DeviceViewModel = {
		row: facts.row,
		presence: facts.presence,
		relationship: facts.relationship,
		keys: facts.keys,
		live: facts.live,
		services: facts.services,
		health: rollupHealth(attention, facts.row, Array.isArray(facts.services)),
		attention,
	};
	if (facts.agent) view.agent = facts.agent;
	if (facts.features) view.features = facts.features;
	const certificates = input.certInventory[deviceId];
	if (certificates) view.certificates = certificates;
	const resources = input.resources[deviceId];
	if (resources) view.resources = resources;
	return view;
}

/* Candidate helpers shared by the rule files. */

type SubjectOf<K extends AttentionSubject["kind"]> = Extract<
	AttentionSubject,
	{ kind: K }
>;

const SUBJECT_IDS: {
	[K in AttentionSubject["kind"]]: (subject: SubjectOf<K>) => string;
} = {
	service: (subject) => `${subject.deviceId}/${subject.serviceId}`,
	device: (subject) => subject.deviceId,
	certificate: (subject) =>
		`${subject.deviceId}/${subject.certificateId ?? "*"}`,
	access: (subject) => `${subject.deviceId}/${subject.personId ?? "me"}`,
	keys: (subject) => subject.deviceId ?? "*",
	setup: (subject) => subject.enrollmentId ?? "*",
	authority: (subject) => subject.authorityId,
	hub: () => "hub",
};

export const subjectId = (subject: AttentionSubject): string =>
	(SUBJECT_IDS[subject.kind] as (subject: AttentionSubject) => string)(subject);

export interface CandidateSpec {
	key: AttentionKey;
	severity: Severity;
	subject: AttentionSubject;
	source: Freshness;
	params?: CopyParams;
	action?: AttentionAction;
	secondary?: Omit<AttentionAction, "gate">;
	lastKnown?: boolean;
	/** When the condition started, if the data says so. */
	since?: number;
	dwellS?: number;
	/** Disambiguates several items of one key on one subject (e.g. logs vs metrics). */
	part?: string;
}

export function attentionCandidate(spec: CandidateSpec): AttentionCandidateExt {
	const id = `${spec.key}:${subjectId(spec.subject)}${spec.part ? `:${spec.part}` : ""}`;
	const candidate: AttentionCandidateExt = {
		id,
		key: spec.key,
		severity: spec.severity,
		subject: spec.subject,
		copy: spec.params
			? { code: spec.key, params: spec.params }
			: { code: spec.key },
		source: spec.source,
		lastKnown: spec.lastKnown ?? false,
	};
	if (spec.action) candidate.action = spec.action;
	if (spec.secondary) candidate.secondary = spec.secondary;
	if (spec.since !== undefined) candidate.since = spec.since;
	if (spec.dwellS !== undefined) candidate.dwellS = spec.dwellS;
	return candidate;
}

export function hubSource(input: AttentionInput): Freshness {
	return classify("device_row", { now: input.now, loaded: true });
}

export function localSource(input: AttentionInput): Freshness {
	return classify("local_vault", { now: input.now, loaded: true });
}

export function deviceRoute(
	deviceId: string,
	tab: DeviceTab,
	certificateId?: string,
): DeviceRoute {
	return certificateId
		? { screen: "device", deviceId, tab, certificateId }
		: { screen: "device", deviceId, tab };
}

export function serviceRoute(
	deviceId: string,
	serviceId: string,
	tab: ServiceTab,
): ServiceRoute {
	return { screen: "service", deviceId, serviceId, tab };
}

export function serviceSubject(view: ServiceView): AttentionSubject {
	return {
		kind: "service",
		deviceId: view.deviceId,
		serviceId: view.serviceId,
		projectId: view.projectId,
	};
}

/** A rule that looks at each device in the hub list. */
export function perDevice(
	key: AttentionKey,
	evaluate: (
		input: AttentionInputExt,
		device: DeviceFacts,
	) => AttentionCandidateExt | AttentionCandidateExt[] | undefined,
): AttentionRuleExt {
	return {
		key,
		evaluate: (input) =>
			fleetFacts(input).devices.flatMap(
				(device) => evaluate(input, device) ?? [],
			),
	};
}

/** Devices whose service list is readable, with their views. */
export function readableServices(
	input: AttentionInputExt,
): { facts: DeviceFacts; services: ServiceView[] }[] {
	return fleetFacts(input).devices.flatMap((facts) =>
		Array.isArray(facts.services) && facts.active
			? [{ facts, services: facts.services }]
			: [],
	);
}

/** Dotted numeric compare; non-numeric parts compare as 0. */
export function compareVersions(a: string, b: string): number {
	const left = a.split(/[.+-]/).map((part) => Number.parseInt(part, 10) || 0);
	const right = b.split(/[.+-]/).map((part) => Number.parseInt(part, 10) || 0);
	for (let index = 0; index < Math.max(left.length, right.length); index++) {
		const diff = (left[index] ?? 0) - (right[index] ?? 0);
		if (diff !== 0) return diff;
	}
	return 0;
}
