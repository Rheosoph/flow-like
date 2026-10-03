import {
	botTokenEventId,
	botTokenKey,
	savedBotToken,
} from "../../../../lib/device-management/bot-config";
import {
	type DeploymentVariable,
	type PlacementConfiguration,
	changedSecretType,
	placementResourcesSchema,
	validateVariableValue,
} from "../../../../lib/device-management/deployment";
import {
	type AppEventInput,
	type AppMode,
	resolvedPin,
} from "../../../../lib/device-management/model/app-plan";
import { buildDeviceView } from "../../../../lib/device-management/model/attention";
import {
	type DeployDraft,
	type DeployPlan,
	type DeployTargetDraft,
	type PlanApp,
	type PlanDevice,
	type PlanFacts,
	type PlanTarget,
	type PlanTargetService,
	isBot,
	resolvePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	deviceName,
	keysLocked,
	rolloutEndsAt,
} from "../../../../lib/device-management/model/device-view";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type {
	AttentionItem,
	DeviceRow,
	DeviceViewModel,
	GateFailure,
	HostIsolationMode,
	InspectionPlus,
	PlacementEvent,
	Presence,
	Relationship,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type {
	KeyState,
	LiveState,
} from "../../../../lib/device-management/workspace/types";
import { parseUint8ArrayToJson } from "../../../../lib/uint8";
import { type GateSources, buildGateContext } from "../workspace/use-attention";
import type { DeployDeviceCheck } from "./step-props";

/* Where-step facts (APP §3.7): one card per device, its first failing gate, and the model's `PlanFacts`. */

export type WhereGateCode =
	| "never"
	| "offline"
	| "nokeys"
	| "no_deploy"
	| "busy"
	| "staged"
	| "other";

export interface WhereGate {
	code: WhereGateCode;
	failure: GateFailure;
	/** `busy`, `staged`: the service whose update is in the way. */
	serviceId?: string;
	/** Unix seconds the device ends that update at the latest. */
	by?: number;
}

export interface DeployDevice {
	id: string;
	name: string;
	row: DeviceRow;
	presence: Presence;
	relationship: Relationship;
	keyState: KeyState;
	/** Keys are here but closed: can be ticked, Continue waits for the unlock. */
	locked: boolean;
	live: LiveState;
	isLive: boolean;
	platform?: string;
	agent?: string;
	isolation?: HostIsolationMode;
	features?: InspectionPlus["features"];
	canManageCertificates: boolean;
	/** Every service on the device; null while no plane can say. */
	services: ServiceView[] | null;
	gate: WhereGate | null;
}

export interface DeployDeviceOptions {
	appId: string | null;
	mode: AppMode | null;
	update: boolean;
	/** Update of one service: its staged update gates the device. */
	serviceId?: string;
}

const ACTIVE = new Set(["validating", "activating", "rolling_back"]);
const LIVE_WAITS = new Set(["connect_first", "connecting"]);

const isLive = (state: LiveState) =>
	state.kind === "live" || state.kind === "renewing";

function synthetic(
	kind: GateFailure["kind"],
	code: GateFailure["copy"]["code"],
	params: Record<string, string | number>,
	fix?: GateFailure["fix"],
): GateFailure {
	return {
		ok: false,
		gate: "G8",
		kind,
		hide: false,
		copy: { code, params },
		...(fix ? { fix } : {}),
	};
}

function presenceGate(view: DeviceViewModel, name: string): WhereGate | null {
	const { kind, since } = view.presence;
	const deviceId = view.row.device_id;
	if (kind === "never")
		return {
			code: "never",
			failure: synthetic(
				"live",
				"never_connected_needs_live",
				{ device: name },
				{ kind: "diagnose", deviceId },
			),
		};
	if (kind !== "offline") return null;
	return {
		code: "offline",
		failure: synthetic(
			"live",
			"offline_needs_live",
			{ device: name, ...(since === undefined ? {} : { since }) },
			{ kind: "diagnose", deviceId },
		),
	};
}

/** What the gate ladder's first failure means on the Where step; null = not a gate here (locked, connecting, platform). */
function failureCode(failure: GateFailure): WhereGateCode | null {
	if (failure.kind === "nokeys") return "nokeys";
	if (failure.gate === "G5") return "no_deploy";
	if (failure.kind === "locked" || failure.kind === "platform") return null;
	if (failure.kind === "busy") return "busy";
	if (failure.kind === "live" && LIVE_WAITS.has(failure.copy.code)) return null;
	return "other";
}

function rolloutGate(
	services: readonly ServiceView[],
	name: string,
	options: DeployDeviceOptions,
): WhereGate | null {
	const own = services.filter((service) => service.projectId === options.appId);
	const busy = own.find(
		(service) => service.rollout && ACTIVE.has(service.rollout.state),
	);
	if (busy?.rollout) {
		const by = rolloutEndsAt(busy.rollout);
		return {
			code: "busy",
			failure: synthetic("busy", "rollout_in_progress", {
				device: name,
				service: busy.serviceId,
			}),
			serviceId: busy.serviceId,
			...(by === undefined ? {} : { by }),
		};
	}
	if (!options.update) return null;
	const staged = own.find(
		(service) =>
			service.rollout?.state === "staged" &&
			(!options.serviceId || service.serviceId === options.serviceId),
	);
	if (!staged?.rollout) return null;
	const by = rolloutEndsAt(staged.rollout);
	return {
		code: "staged",
		failure: synthetic("busy", "rollout_in_progress", {
			device: name,
			service: staged.serviceId,
		}),
		serviceId: staged.serviceId,
		...(by === undefined ? {} : { by }),
	};
}

function deviceGate(
	sources: GateSources,
	view: DeviceViewModel,
	name: string,
	options: DeployDeviceOptions,
): WhereGate | null {
	const seen = presenceGate(view, name);
	if (seen) return seen;
	const result = evaluateGate(
		options.update ? "update_service" : "create_service",
		buildGateContext(sources, view.row.device_id, {
			...(options.appId ? { projectId: options.appId } : {}),
			extra: { offlineLocalApp: options.mode === "offline" },
		}),
	);
	if (!result.ok) {
		const code = failureCode(result);
		if (code) return { code, failure: result };
	}
	return Array.isArray(view.services)
		? rolloutGate(view.services, name, options)
		: null;
}

function deployDevice(
	sources: GateSources,
	view: DeviceViewModel,
	options: DeployDeviceOptions,
): DeployDevice {
	const { row } = view;
	const name = deviceName(row);
	const read = sources.input.live[row.device_id];
	const inspection = read?.inspection?.value;
	const agent = view.agent?.version;
	const platform = inspection?.isolation?.platform;
	const isolation = inspection?.hostIsolation ?? undefined;
	// The live read's flags, else the ones of the status the services come from; none = unknown.
	const features = inspection?.features ?? view.features;
	return {
		id: row.device_id,
		name,
		row,
		presence: view.presence,
		relationship: view.relationship,
		keyState: view.keys.state,
		locked: keysLocked(view.keys),
		live: view.live,
		isLive: isLive(view.live),
		...(platform ? { platform } : {}),
		...(agent ? { agent } : {}),
		...(isolation ? { isolation } : {}),
		...(features ? { features } : {}),
		canManageCertificates:
			inspection?.certificate_management === 1 &&
			inspection.can_manage_certificates === true,
		services: Array.isArray(view.services) ? view.services : null,
		gate: deviceGate(sources, view, name, options),
	};
}

/** Every device that isn't revoked, in hub order, with its card facts and gate; revoked ones are only counted. */
export function buildDeployDevices(
	sources: GateSources,
	items: AttentionItem[],
	options: DeployDeviceOptions,
): { devices: DeployDevice[]; revoked: number } {
	const devices: DeployDevice[] = [];
	let revoked = 0;
	for (const row of sources.input.devices) {
		if (row.status === "revoked") {
			revoked += 1;
			continue;
		}
		const view = buildDeviceView(row.device_id, sources.input, items);
		if (view) devices.push(deployDevice(sources, view, options));
	}
	return { devices, revoked };
}

/** Ports the device's services listen on, from their configurations read live. */
export function portsInUse(
	configurations: readonly PlacementConfiguration[],
): NonNullable<PlanDevice["portsInUse"]>[number][] {
	return configurations.flatMap((configuration) =>
		configuration.config.hosting
			? [
					{
						port: configuration.config.hosting.port,
						serviceId: configuration.placement_id,
					},
				]
			: [],
	);
}

export interface PlanDeviceExtras {
	configurations?: readonly PlacementConfiguration[];
	check?: DeployDeviceCheck;
}

export function planDevice(
	device: DeployDevice,
	extras: PlanDeviceExtras = {},
): PlanDevice {
	const ports = portsInUse(extras.configurations ?? []);
	return {
		id: device.id,
		name: device.name,
		gate: device.gate?.failure ?? null,
		locked: device.locked,
		services:
			device.services?.map((service) => ({
				serviceId: service.serviceId,
				projectId: service.projectId,
				events: service.events?.map((event) => event.event_id) ?? null,
				desired: service.desired,
				maxInstances: service.instances.max,
			})) ?? null,
		...(extras.check ? { refusals: extras.check.refusals } : {}),
		...(ports.length ? { portsInUse: ports } : {}),
		...(device.isolation ? { isolation: device.isolation } : {}),
		...(device.features ? { features: device.features } : {}),
	};
}

export interface PlanFactsInput
	extends Pick<
		PlanFacts,
		"hub" | "schedules" | "canEditEvents" | "localTriggers"
	> {
	app: PlanApp | null;
	devices: readonly DeployDevice[];
	configurations: Readonly<Record<string, readonly PlacementConfiguration[]>>;
	checks: Readonly<Record<string, DeployDeviceCheck>>;
	platform: "desktop" | "web";
	now: number;
	isAppOwner?: boolean;
}

export function buildPlanFacts(input: PlanFactsInput): PlanFacts {
	return {
		app: input.app,
		devices: Object.fromEntries(
			input.devices.map((device) => [
				device.id,
				planDevice(device, {
					configurations: input.configurations[device.id],
					check: input.checks[device.id],
				}),
			]),
		),
		platform: input.platform,
		now: input.now,
		...(input.isAppOwner === undefined ? {} : { isAppOwner: input.isAppOwner }),
		...(input.hub ? { hub: input.hub } : {}),
		...(input.schedules === undefined ? {} : { schedules: input.schedules }),
		...(input.canEditEvents === undefined
			? {}
			: { canEditEvents: input.canEditEvents }),
		...(input.localTriggers?.length
			? { localTriggers: input.localTriggers }
			: {}),
	};
}

const samePin = (now: readonly number[], pinned: readonly number[]) =>
	now.join(".") === pinned.join(".");

/**
 * A service whose events are all pinned to what the app publishes now: an
 * update has nothing to upload. An event that follows Latest counts by the
 * flow version that equals its flow; unknown, and flow edits no version holds,
 * count as no.
 */
export function runsNewest(
	events: PlanApp["events"],
	served: readonly PlacementEvent[] | null | undefined,
): boolean {
	if (!served?.length) return false;
	return served.every((pin) => {
		const event = events.find((row) => row.id === pin.event_id);
		const newest = event ? resolvedPin(event) : null;
		return (
			!!newest &&
			samePin(newest.eventVersion, pin.event_version) &&
			samePin(newest.boardVersion, pin.board_version)
		);
	});
}

/** A sandboxed target whose limits a device refuses; the bounds are the lib's own. */
export function limitsOutOfRange(plan: DeployPlan): boolean {
	return plan.targets.some(
		(target) =>
			target.resources != null &&
			!placementResourcesSchema.safeParse(target.resources).success,
	);
}

/** The settings the given events use, one row per variable, by name. */
export function eventVariables(
	app: PlanApp | null,
	events: readonly string[],
): DeploymentVariable[] {
	const rows = new Map<string, DeploymentVariable>();
	for (const eventId of events)
		for (const variable of app?.variables?.[eventId] ?? [])
			if (!rows.has(variable.id)) rows.set(variable.id, variable);
	return [...rows.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** The chosen events that use a variable ("Used by"). */
export function variableUsers(
	app: PlanApp | null,
	events: readonly string[],
	variableId: string,
): string[] {
	return events.filter((eventId) =>
		(app?.variables?.[eventId] ?? []).some((row) => row.id === variableId),
	);
}

type EventVariables = Readonly<Record<string, readonly DeploymentVariable[]>>;

/**
 * Variable definitions per event, the newest source last: what the services
 * run now, what devices discovered in the new copy, the approved bundle.
 */
export function mergeEventVariables(
	installed: EventVariables | undefined,
	checks: Readonly<Record<string, DeployDeviceCheck>>,
	approved: EventVariables | undefined,
): Record<string, readonly DeploymentVariable[]> | undefined {
	const merged: Record<string, readonly DeploymentVariable[]> = {
		...installed,
	};
	for (const check of Object.values(checks))
		Object.assign(merged, check.variables ?? {});
	Object.assign(merged, approved ?? {});
	return Object.keys(merged).length ? merged : undefined;
}

export type StoredOverrideIssue =
	| "outside"
	| "kind_changed"
	| "type_changed"
	| "invalid";

/** A value an updated service has stored that the update can't carry over as it is. */
export interface UnresolvedOverride {
	deviceId: string;
	device: string;
	serviceId: string;
	variableId: string;
	/** The variable's name when it still has a definition. */
	name: string;
	secret: boolean;
	issue: StoredOverrideIssue;
	/** `type_changed`: what the stored secret was written for. */
	previous?: DeploymentVariable;
	current?: DeploymentVariable;
}

export interface OverrideFacts {
	configurations: Readonly<Record<string, readonly PlacementConfiguration[]>>;
	/** Definitions the stored secrets were written for, by `${deviceId}/${serviceId}`. */
	previous: Readonly<Record<string, readonly DeploymentVariable[]>>;
}

function replaced(
	draft: DeployDraft,
	target: DeployTargetDraft | undefined,
	variable: DeploymentVariable,
): boolean {
	if (!draft.edited.includes(variable.id)) return false;
	if (!variable.secret)
		return (
			draft.vars[variable.id] !== undefined ||
			target?.over.vars?.[variable.id] !== undefined
		);
	const shared =
		draft.secretsMode === "same" ? draft.secrets[variable.id] : undefined;
	return Boolean(target?.over.secrets?.[variable.id] ?? shared);
}

function storedIssue(
	existing: PlacementConfiguration,
	variable: DeploymentVariable | undefined,
	id: string,
	context: { replaced: boolean; previous?: readonly DeploymentVariable[] },
): Pick<UnresolvedOverride, "issue" | "previous"> | null {
	if (!variable) return { issue: "outside" };
	const wasSecret = Object.hasOwn(existing.config.secret_overrides, id);
	if (context.replaced) return null;
	if (wasSecret !== variable.secret) return { issue: "kind_changed" };
	if (wasSecret) {
		const previous = changedSecretType([...(context.previous ?? [])], variable);
		return previous ? { issue: "type_changed", previous } : null;
	}
	try {
		validateVariableValue(variable, existing.config.variables[id]);
		return null;
	} catch {
		return { issue: "invalid" };
	}
}

function targetUnresolved(
	plan: DeployPlan,
	target: PlanTarget,
	service: PlanTargetService,
	facts: OverrideFacts,
): UnresolvedOverride[] {
	const existing = facts.configurations[target.deviceId]?.find(
		(row) => row.placement_id === service.serviceId,
	);
	// Without the definitions of the events it will serve, nothing can be said yet.
	if (!existing || !plan.app?.variables) return [];
	const own = plan.draft.targets.find(
		(row) => row.deviceId === target.deviceId,
	);
	const removed = new Set(own?.over.removeOverrides ?? []);
	const definitions = eventVariables(plan.app, service.events);
	const previous = facts.previous[`${target.deviceId}/${service.serviceId}`];
	const stored = [
		...Object.keys(existing.config.variables),
		...Object.keys(existing.config.secret_overrides),
	];
	// The stored token of a bot the update drops is removed with it (`wirePlan`): nothing to decide.
	const dropped = new Set(service.removedEvents);
	return stored.flatMap((id) => {
		if (removed.has(id) || dropped.has(botTokenEventId(id) ?? "")) return [];
		const variable = definitions.find((row) => row.id === id);
		const found = storedIssue(existing, variable, id, {
			replaced: variable ? replaced(plan.draft, own, variable) : false,
			previous,
		});
		if (!found) return [];
		return [
			{
				deviceId: target.deviceId,
				device: target.name,
				serviceId: service.serviceId,
				variableId: id,
				name: variable?.name ?? id,
				secret: Object.hasOwn(existing.config.secret_overrides, id),
				...found,
				...(variable ? { current: variable } : {}),
			},
		];
	});
}

/**
 * Stored values of updated services that need the user: set for an event the
 * update no longer serves, switched between plain and secret, a secret written
 * for another type, or a value its type no longer accepts. The device refuses
 * such an update, so Settings asks before Review does.
 */
export function unresolvedOverrides(
	plan: DeployPlan,
	facts: OverrideFacts,
): UnresolvedOverride[] {
	return plan.targets.flatMap((target) =>
		target.services
			.filter((service) => service.kind !== "new")
			.flatMap((service) => targetUnresolved(plan, target, service, facts)),
	);
}

/** The token saved on a bot event's record, for Settings only; null for any other event. */
export function savedTokenOf(event: AppEventInput | undefined): string | null {
	if (!event || !isBot(event) || !event.config?.length) return null;
	return savedBotToken(
		event.event_type,
		parseUint8ArrayToJson(event.config as number[]),
	);
}

/** Whether the person entered a token for this bot, shared or for one device (an empty one counts: "Enter a token"). */
export function enteredBotToken(draft: DeployDraft, eventId: string): boolean {
	const key = botTokenKey(eventId);
	return (
		draft.secrets[key] !== undefined ||
		draft.targets.some((target) => target.over.secrets?.[key] !== undefined)
	);
}

function withToken(
	draft: DeployDraft,
	key: string,
	token: string,
	deviceIds: ReadonlySet<string>,
): DeployDraft {
	if (draft.secretsMode === "same")
		return { ...draft, secrets: { ...draft.secrets, [key]: token } };
	return {
		...draft,
		targets: draft.targets.map((target) =>
			deviceIds.has(target.deviceId)
				? {
						...target,
						over: {
							...target.over,
							secrets: { ...target.over.secrets, [key]: token },
						},
					}
				: target,
		),
	};
}

/**
 * "Use the token saved in Events", the default of a bot whose record has a
 * token: for every device this deploy adds the bot to, the record's token
 * stands where the plan reads typed secrets, as long as nobody entered one.
 * It lives in the plan only: the saved draft never holds it, and it leaves
 * this computer only inside `set_secret` or `rollout_secret`. A service that
 * keeps its bot keeps its stored token.
 */
export function withSavedBotTokens(
	draft: DeployDraft,
	facts: PlanFacts,
): DeployDraft {
	const saved = (facts.app?.events ?? []).flatMap((event) => {
		const token = draft.events.includes(event.id) ? savedTokenOf(event) : null;
		return token && !enteredBotToken(draft, event.id)
			? [{ eventId: event.id, token }]
			: [];
	});
	if (!saved.length) return draft;
	const plan = resolvePlan(draft, facts);
	let next = draft;
	for (const { eventId, token } of saved) {
		const adding = new Set(
			plan.targets
				.filter((target) =>
					target.services.some((service) =>
						service.addedBots.includes(eventId),
					),
				)
				.map((target) => target.deviceId),
		);
		if (adding.size)
			next = withToken(next, botTokenKey(eventId), token, adding);
	}
	return next;
}
