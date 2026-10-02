import { z } from "zod";
import {
	type CreateResourceGrant,
	MAX_BILLING_MICROS,
} from "../../device-resources";
import {
	type DeploymentEvent,
	type DeploymentVariable,
	type InstalledProject,
	type OfflineWritesConfig,
	type PlacementConfiguration,
	type PlacementResources,
	canCheckDeploymentStartup,
	type createDeploymentPlan,
	eventEligibility,
	mergeVariables,
	offlineWritesSchema,
	variableValue,
} from "../deployment";
import {
	type AppEventInput,
	type AppInput,
	type AppMode,
	appMode,
} from "./app-plan";
import type {
	CopyParams,
	DeployRoute,
	DeployStepId,
	DevicesScope,
	GateFailure,
	HostIsolationMode,
	OnlineAccess,
} from "./types";

/* One plan for every deploy entry (APP §3.3, A4/A5): the draft holds choices, the plan derives services and targets. */

export type DeployEntryKind = "app" | "event" | "device" | "update";
export type DeployOrder = "one" | "all" | "first";
export type DeployStrategy = "auto" | "safe" | "quick";
export type ServiceSplit = "one" | "per_event";
export type TokenMode = "per_device" | "same" | "own" | "keep";
export type IsolationProfile = "auto" | "linux_sandbox" | "trusted_process";

export interface IsolationDraft {
	profile: IsolationProfile;
	cpuMillis: number;
	memoryBytes: number;
	maxProcesses: number;
	diskBytes: number;
}

export const DEFAULT_ISOLATION: IsolationDraft = {
	profile: "auto",
	cpuMillis: 1000,
	memoryBytes: 1024 ** 3,
	maxProcesses: 256,
	diskBytes: 4 * 1024 ** 3,
};

/** Inline cloud approval (APP §3.10); one per service on each device. */
export interface ApprovalDraft {
	files: "none" | OnlineAccess;
	/** "I own the app and allow these services to read and change its files." */
	ownerConsent: boolean;
	models: string[];
	maxInstances: number;
	/** Unix seconds. */
	expiresAt: number;
}

/** Spending limit for the approval's models; the payer is the approver. */
export interface SpendingDraft {
	limitMicros: number;
	/** Unix seconds; never after the approval ends. */
	expiresAt: number;
	consent: boolean;
}

export interface EndpointDraft {
	/** null keeps each service's current address (updates). */
	host: string | null;
	port: number | null;
	token: TokenMode;
	/** The shared or typed token for `same` / `own`. */
	tokenValue: string;
}

export interface DeployOverrides {
	vars?: Record<string, string>;
	secrets?: Record<string, string>;
	host?: string;
	port?: number;
	/** null = no certificate; undefined = keep the service's current one. */
	certificateId?: string | null;
	token?: string;
	isolation?: IsolationDraft;
	spendingLimitMicros?: number;
	allowUnencrypted?: boolean;
	/** "{device} runs {app} with the agent's full access." */
	trustAgent?: boolean;
	removeOverrides?: string[];
}

export type TargetChoice =
	| { kind: "new" }
	| { kind: "update" | "add"; serviceId: string };

export interface DeployTargetDraft {
	deviceId: string;
	/** By planned service key; absent = a new service. */
	choices: Record<string, TargetChoice>;
	/** Events served here although another service of the app already serves them. */
	serveBoth: string[];
	over: DeployOverrides;
}

export interface DeployDraft {
	entry: DeployEntryKind;
	ctx: "app" | "events" | null;
	appId: string | null;
	version: "newest" | "keep";
	scope: "app" | "events" | "event";
	events: string[];
	split: ServiceSplit;
	/** Service ids the user typed, by planned service key. */
	serviceIds: Record<string, string>;
	maxInstances: number;
	targets: DeployTargetDraft[];
	vars: Record<string, string>;
	/** Values of secret variables live only here and in `over.secrets`: `draftWithoutSecrets` drops exactly these. */
	secrets: Record<string, string>;
	secretsMode: "same" | "per_device";
	/** Variable ids changed in this draft; updates send only these. */
	edited: string[];
	endpoint: EndpointDraft;
	/** undefined keeps each service's limits (updates). */
	isolation?: IsolationDraft;
	approval: ApprovalDraft;
	spending: SpendingDraft | null;
	/** undefined keeps each service's write buffering (updates); null turns it off. */
	writes?: OfflineWritesConfig | null;
	start: boolean;
	order: DeployOrder;
	stopOnFail: boolean;
	strategy: DeployStrategy;
	/** One deployment id per plan, reused on every device. */
	deploymentId: string;
	acceptRemovedEvents: boolean;
	/**
	 * Version-only update of several services (APP §2.16, §3.14): every updated
	 * service keeps the events it serves now. `events` then only says which of
	 * them the newest version still has; nothing is added and nothing is split.
	 */
	keepEvents?: boolean;
}

export interface PlanApp extends Pick<AppInput, "id" | "name" | "visibility"> {
	events: readonly AppEventInput[];
	/** Definitions per event once known (approved bundle or the device's discovery). */
	variables?: Readonly<Record<string, readonly DeploymentVariable[]>>;
}

export interface PlanDeviceService {
	serviceId: string;
	projectId: string;
	/** null when the plane carries no event list. */
	events: readonly string[] | null;
	desired?: string;
}

export interface PlanDevice {
	id: string;
	name: string;
	/** First failing Where gate (APP §3.7); locked is not a gate. */
	gate: GateFailure | null;
	locked: boolean;
	/** null while unknown (locked). */
	services: readonly PlanDeviceService[] | null;
	reservedIds?: readonly string[];
	refusals?: Readonly<Record<string, string>>;
	portsInUse?: readonly { port: number; serviceId?: string }[];
	isolation?: HostIsolationMode;
	memoryBytes?: number;
}

export interface PlanFacts {
	app: PlanApp | null;
	devices: Readonly<Record<string, PlanDevice>>;
	platform: "desktop" | "web";
	now: number;
	/** Online files need the app owner (`repository.rs:213-228`); undefined = unknown. */
	isAppOwner?: boolean;
}

export type ServiceWhy =
	| { code: "background"; eventId: string }
	| { code: "writes" }
	| { code: "split_variables"; variable: string; events: [string, string] };

export interface PlannedService {
	key: string;
	id: string;
	events: string[];
	maxInstances: number;
	hosted: boolean;
	why: ServiceWhy[];
}

export type LeftOut =
	| { eventId: string; why: "refuse"; detail: string }
	| { eventId: string; why: "duplicate"; serviceId: string };

export interface PlanTargetService {
	key: string;
	serviceId: string;
	kind: TargetChoice["kind"];
	events: string[];
	leftOut: LeftOut[];
	/** The default id was taken on this device. */
	renamedFrom?: string;
	/** Events the update drops (APP §3.5 item 6). */
	removedEvents: string[];
	/** Set when this service can't use the target's port: another served service of the plan has it (A5). */
	port?: number;
}

export interface PlanTarget {
	deviceId: string;
	name: string;
	gate: GateFailure | null;
	locked: boolean;
	services: PlanTargetService[];
	endpoint: {
		/** null = keep the current address. */
		host: string | null;
		port: number | null;
		portMovedFrom?: number;
		/** undefined = keep the current certificate. */
		certificateId?: string | null;
	};
	/** undefined = keep; null = runs as the agent. */
	resources?: PlacementResources | null;
	runsAsAgent: boolean;
}

export interface DeployPlan {
	draft: DeployDraft;
	app: PlanApp | null;
	mode: AppMode | null;
	services: PlannedService[];
	targets: PlanTarget[];
}

export const APPROVAL_ISSUE_CODES = [
	"models_invalid",
	"nothing_approved",
	"files_need_app",
	"files_need_owner",
	"files_need_consent",
	"instances_range",
	"instances_below_service",
	"expiry_past",
	"expiry_too_far",
	"spending_required",
	"spending_without_models",
	"spending_range",
	"spending_expiry",
	"spending_consent",
] as const;
export type ApprovalIssueCode = (typeof APPROVAL_ISSUE_CODES)[number];

export interface ApprovalIssue {
	code: ApprovalIssueCode;
	field: "files" | "models" | "maxInstances" | "expiresAt" | "spending";
	params?: CopyParams;
}

export const PLAN_ISSUE_CODES = [
	"app_missing",
	"local_only_web",
	"no_events",
	"too_many_events",
	"service_id_invalid",
	"service_id_twice",
	"service_id_reserved",
	"removed_events",
	"no_targets",
	"target_gated",
	"target_locked",
	"target_no_events",
	"variable_invalid",
	"secret_size",
	"service_instances_range",
	"update_needs_service",
	"host_invalid",
	"port_invalid",
	"port_in_use",
	"token_invalid",
	"unencrypted",
	"writes_offline",
	"writes_invalid",
	"isolation_unavailable",
	"isolation_required",
	"agent_trust",
] as const;
export type PlanIssueCode =
	| (typeof PLAN_ISSUE_CODES)[number]
	| `approval.${ApprovalIssueCode}`;

export interface PlanIssue {
	code: PlanIssueCode;
	step: DeployStepId;
	severity: "error" | "warning";
	deviceId?: string;
	serviceKey?: string;
	params?: CopyParams;
}

export type PlanExceptionCode =
	| "left_out_refuse"
	| "left_out_duplicate"
	| "renamed"
	| "port_moved"
	| "no_certificate"
	| "runs_as_agent";

/** APP §6.3 exceptions rows: `warning` needs an acknowledgement, `info` differs by choice, `paused` is left out. */
export interface PlanException {
	code: PlanExceptionCode;
	step: DeployStepId;
	deviceId: string;
	tone: "warning" | "info" | "paused";
	params?: CopyParams;
}

export interface PlanCheck {
	ok: boolean;
	issues: PlanIssue[];
	exceptions: PlanException[];
	/** First error in step order: the Review button's reason and link. */
	firstBlocking: PlanIssue | null;
}

export type DeployPhase =
	| "approve"
	| "spending"
	| "upload"
	| "check_events"
	| "install"
	| "create"
	| "secrets"
	| "start"
	| "prepare_update"
	| "check_new"
	| "switch"
	| "stop";

export interface DeployFailure {
	phase: DeployPhase;
	/** Machine code from the device or client (`DeploymentRejectedError.code`, `failure_code`, …). */
	code: string;
	detail?: string;
	/** An update that switched over and was restored by the device. */
	rolledBack?: boolean;
}

export interface DeployResult {
	outcome: "all" | "partial" | "none";
	at: number;
	done: string[];
	failed: (DeployFailure & { target: string })[];
	skipped: string[];
	notStarted: string[];
	/** The shared phase failed before any device started. */
	sharedFailure?: DeployFailure;
}

export type DeploymentPlanInput = Parameters<typeof createDeploymentPlan>[0];

const SERVICE_ID = /^[A-Za-z0-9_.-]{1,128}$/;
const MAX_EVENTS_PER_SERVICE = 64;
const MAX_APPROVAL_SECONDS = 365 * 86400;
const DEFAULT_APPROVAL_SECONDS = 30 * 86400;
const TOKEN = /^[\x21-\x7e]{32,4096}$/;
const STEP_ORDER: readonly DeployStepId[] = [
	"what",
	"how",
	"where",
	"settings",
	"endpoint",
	"access_cost",
	"copy_upload",
	"review",
	"rollout",
];

export function isServiceId(id: string): boolean {
	return SERVICE_ID.test(id) && id !== "." && id !== ".." && id !== "device";
}

/** Default service id from a name: lower-case words joined by `-`. */
export function serviceSlug(text: string): string {
	const slug = text
		.normalize("NFKD")
		.replace(/\p{M}/gu, "")
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/^-+|-+$/g, "")
		.slice(0, 120);
	return isServiceId(slug) ? slug : "service";
}

function isHosted(event: AppEventInput): boolean {
	return eventEligibility(event).hosted;
}

function eligibleEvents(app: PlanApp | null): AppEventInput[] {
	return (app?.events ?? []).filter(
		(event) => eventEligibility(event).eligible,
	);
}

/* Entries (APP §3.1). */

export interface PlanEntry {
	scope: DevicesScope;
	route: Pick<
		DeployRoute,
		"deviceIds" | "appId" | "serviceId" | "mode" | "eventId" | "from"
	>;
	app: PlanApp | null;
	deploymentId: string;
	now: number;
	/** Update entries: the events the routed service serves now. */
	updateEvents?: readonly string[];
}

function entryKind(entry: PlanEntry) {
	const { route } = entry;
	if (route.serviceId || route.mode === "update") return "update";
	if (route.eventId) return "event";
	return entry.scope.kind === "app" || !route.deviceIds.length
		? "app"
		: "device";
}

function entryEvents(entry: PlanEntry, kind: DeployEntryKind) {
	if (entry.route.eventId) return [entry.route.eventId];
	if (kind === "update" && entry.updateEvents) return [...entry.updateEvents];
	return eligibleEvents(entry.app).map((event) => event.id);
}

function entryTargets(
	entry: PlanEntry,
	kind: DeployEntryKind,
): DeployTargetDraft[] {
	const { serviceId } = entry.route;
	const choices: Record<string, TargetChoice> =
		kind === "update" && serviceId
			? { main: { kind: "update", serviceId } }
			: {};
	return [...new Set(entry.route.deviceIds)].map((deviceId) => ({
		deviceId,
		choices: { ...choices },
		serveBoth: [],
		over: {},
	}));
}

/** The starting draft for an entry; prefilled steps stay editable. */
export function makePlan(entry: PlanEntry): DeployDraft {
	const kind = entryKind(entry);
	const update = kind === "update";
	const mode = entry.app ? appMode(entry.app.visibility) : null;
	return {
		entry: kind,
		ctx:
			entry.scope.kind === "app"
				? entry.route.from === "events"
					? "events"
					: "app"
				: null,
		appId:
			entry.route.appId ??
			(entry.scope.kind === "app" ? entry.scope.appId : null),
		version: "newest",
		scope: entry.route.eventId ? "event" : "app",
		events: entryEvents(entry, kind),
		split: "one",
		serviceIds: {},
		maxInstances: 1,
		targets: entryTargets(entry, kind),
		vars: {},
		secrets: {},
		secretsMode: "same",
		edited: [],
		endpoint: {
			host: update ? null : "127.0.0.1",
			port: update ? null : 8080,
			token: update ? "keep" : "per_device",
			tokenValue: "",
		},
		...(update ? {} : { isolation: DEFAULT_ISOLATION, writes: null }),
		...(update && !entry.route.eventId && !entry.route.serviceId
			? { keepEvents: true }
			: {}),
		approval: {
			files: mode === "online" ? "read_only" : "none",
			ownerConsent: false,
			models: [],
			maxInstances: 1,
			expiresAt: entry.now + DEFAULT_APPROVAL_SECONDS,
		},
		spending: null,
		start: true,
		order: "one",
		stopOnFail: true,
		strategy: "auto",
		deploymentId: entry.deploymentId,
		acceptRemovedEvents: false,
	};
}

/** The draft as saved progress may keep it (APP §3.1): every choice, no secret value and no typed token. */
export function draftWithoutSecrets(draft: DeployDraft): DeployDraft {
	return {
		...draft,
		secrets: {},
		endpoint: { ...draft.endpoint, tokenValue: "" },
		targets: draft.targets.map((target) => {
			const { secrets: _secrets, token: _token, ...over } = target.over;
			return { ...target, over };
		}),
	};
}

/* Services (A5): one service, split only where the code forces it. */

function eventVariables(
	app: PlanApp,
	eventId: string,
): readonly DeploymentVariable[] {
	return app.variables?.[eventId] ?? [];
}

function conflictWith(
	group: AppEventInput[],
	app: PlanApp,
	event: AppEventInput,
): ServiceWhy | null {
	for (const other of group)
		for (const variable of eventVariables(app, event.id)) {
			const existing = eventVariables(app, other.id).find(
				(value) => value.id === variable.id,
			);
			if (existing && JSON.stringify(existing) !== JSON.stringify(variable))
				return {
					code: "split_variables",
					variable: variable.name,
					events: [other.id, event.id],
				};
		}
	return null;
}

function groupEvents(
	events: AppEventInput[],
	app: PlanApp,
	split: ServiceSplit,
): { events: AppEventInput[]; why: ServiceWhy[] }[] {
	if (split === "per_event")
		return events.map((event) => ({ events: [event], why: [] }));
	const groups: { events: AppEventInput[]; why: ServiceWhy[] }[] = [];
	for (const event of events) {
		const conflicts = groups.map((group) =>
			conflictWith(group.events, app, event),
		);
		const fit = conflicts.findIndex((conflict) => conflict === null);
		if (fit >= 0) groups[fit].events.push(event);
		else
			groups.push({
				events: [event],
				why: conflicts.filter((value): value is ServiceWhy => value !== null),
			});
	}
	return groups;
}

function defaultServiceId(
	draft: DeployDraft,
	app: PlanApp,
	first: AppEventInput,
	index: number,
): string {
	const byEvent =
		draft.scope === "event" || draft.split === "per_event" || index > 0;
	return serviceSlug(byEvent ? first.name : app.name);
}

function instanceLimit(
	draft: DeployDraft,
	events: AppEventInput[],
): { max: number; why: ServiceWhy[] } {
	const background = events.find((event) => !isHosted(event));
	const why: ServiceWhy[] = [
		...(background
			? [{ code: "background", eventId: background.id } as const]
			: []),
		...(draft.writes ? [{ code: "writes" } as const] : []),
	];
	const requested = Math.min(32, Math.max(1, Math.trunc(draft.maxInstances)));
	return { max: why.length ? 1 : requested, why };
}

export function planServices(
	draft: DeployDraft,
	app: PlanApp | null,
): PlannedService[] {
	if (!app) return [];
	const chosen = eligibleEvents(app).filter((event) =>
		draft.events.includes(event.id),
	);
	// Services that keep their events were split when they were created: one entry stands for all of them.
	const groups = draft.keepEvents
		? [{ events: chosen, why: [] }]
		: groupEvents(chosen, app, draft.split);
	return groups.map((group, index) => {
		const [first] = group.events;
		const single = draft.keepEvents || (draft.split === "one" && index === 0);
		const key = single ? "main" : (first?.id ?? "main");
		const limit = instanceLimit(draft, group.events);
		return {
			key,
			id: draft.serviceIds[key] ?? defaultServiceId(draft, app, first, index),
			events: group.events.map((event) => event.id),
			maxInstances: limit.max,
			hosted: group.events.some(isHosted),
			why: [...group.why, ...limit.why],
		};
	});
}

/* Targets (APP §3.7). */

function freeId(base: string, taken: ReadonlySet<string>): string {
	if (!taken.has(base)) return base;
	let n = 2;
	while (taken.has(`${base}-${n}`)) n++;
	return `${base}-${n}`;
}

function appServicesOn(
	device: PlanDevice | undefined,
	appId: string | null,
): readonly PlanDeviceService[] {
	return (device?.services ?? []).filter(
		(service) => service.projectId === appId,
	);
}

function leftOutOn(
	events: string[],
	device: PlanDevice | undefined,
	others: readonly PlanDeviceService[],
	serveBoth: readonly string[],
): { kept: string[]; leftOut: LeftOut[] } {
	const kept: string[] = [];
	const leftOut: LeftOut[] = [];
	for (const eventId of events) {
		const refusal = device?.refusals?.[eventId];
		const servedBy = serveBoth.includes(eventId)
			? undefined
			: others.find((service) => service.events?.includes(eventId));
		if (refusal) leftOut.push({ eventId, why: "refuse", detail: refusal });
		else if (servedBy)
			leftOut.push({
				eventId,
				why: "duplicate",
				serviceId: servedBy.serviceId,
			});
		else kept.push(eventId);
	}
	return { kept, leftOut };
}

function targetService(
	service: PlannedService,
	target: DeployTargetDraft,
	device: PlanDevice | undefined,
	{ appId, keepEvents }: Pick<DeployDraft, "appId" | "keepEvents">,
	planned: ReadonlySet<string>,
): PlanTargetService {
	const choice = target.choices[service.key] ?? { kind: "new" };
	const appServices = appServicesOn(device, appId);
	if (choice.kind === "new") {
		const base = service.id;
		const serviceId = freeId(
			base,
			new Set([
				...(device?.services ?? []).map((value) => value.serviceId),
				...planned,
			]),
		);
		const { kept, leftOut } = leftOutOn(
			service.events,
			device,
			appServices,
			target.serveBoth,
		);
		return {
			key: service.key,
			serviceId,
			kind: "new",
			events: kept,
			leftOut,
			removedEvents: [],
			...(serviceId === base ? {} : { renamedFrom: base }),
		};
	}
	const existing = appServices.find(
		(value) => value.serviceId === choice.serviceId,
	);
	const current = existing?.events ?? [];
	if (keepEvents && choice.kind === "update") {
		// Its own events that the newest version still has; one that is gone is a removed event to accept.
		const kept = current.filter((eventId) => service.events.includes(eventId));
		return {
			key: service.key,
			serviceId: choice.serviceId,
			kind: "update",
			events: kept,
			leftOut: [],
			removedEvents: current.filter((eventId) => !kept.includes(eventId)),
		};
	}
	const wanted =
		choice.kind === "add"
			? [...new Set([...current, ...service.events])]
			: service.events;
	const { kept, leftOut } = leftOutOn(
		wanted,
		device,
		appServices.filter((value) => value.serviceId !== choice.serviceId),
		[...target.serveBoth, ...current],
	);
	return {
		key: service.key,
		serviceId: choice.serviceId,
		kind: choice.kind,
		events: kept,
		leftOut,
		removedEvents: current.filter((eventId) => !kept.includes(eventId)),
	};
}

function nextFreePort(
	port: number,
	device: PlanDevice | undefined,
	own: readonly string[],
): number {
	const used = new Set(
		(device?.portsInUse ?? [])
			.filter((row) => !row.serviceId || !own.includes(row.serviceId))
			.map((row) => row.port),
	);
	let candidate = port;
	while (used.has(candidate) && candidate < 65535) candidate++;
	return candidate;
}

function targetCertificate(
	target: DeployTargetDraft,
	services: readonly PlanTargetService[],
): Pick<PlanTarget["endpoint"], "certificateId"> {
	if ("certificateId" in target.over)
		return { certificateId: target.over.certificateId };
	return services.every((service) => service.kind === "new")
		? { certificateId: null }
		: {};
}

function targetEndpoint(
	draft: DeployDraft,
	target: DeployTargetDraft,
	device: PlanDevice | undefined,
	services: readonly PlanTargetService[],
): PlanTarget["endpoint"] {
	const host = target.over.host ?? draft.endpoint.host;
	const wanted = target.over.port ?? draft.endpoint.port;
	const port =
		wanted === null || target.over.port !== undefined
			? wanted
			: nextFreePort(
					wanted,
					device,
					services.map((service) => service.serviceId),
				);
	return {
		host,
		port,
		...(port !== wanted && wanted !== null ? { portMovedFrom: wanted } : {}),
		...targetCertificate(target, services),
	};
}

function sandboxLimits(
	isolation: IsolationDraft,
	device: PlanDevice | undefined,
): PlacementResources {
	return {
		profile: "linux_sandbox",
		cpu_millis: isolation.cpuMillis,
		memory_bytes: device?.memoryBytes
			? Math.min(isolation.memoryBytes, device.memoryBytes)
			: isolation.memoryBytes,
		max_processes: isolation.maxProcesses,
		disk_bytes: isolation.diskBytes,
	};
}

function targetResources(
	isolation: IsolationDraft | undefined,
	device: PlanDevice | undefined,
): Pick<PlanTarget, "resources" | "runsAsAgent"> {
	if (!isolation) return { runsAsAgent: false };
	const canSandbox =
		device?.isolation === "required" || device?.isolation === "optional";
	const sandbox =
		isolation.profile === "linux_sandbox" ||
		(isolation.profile === "auto" && canSandbox);
	return sandbox
		? { resources: sandboxLimits(isolation, device), runsAsAgent: false }
		: { resources: null, runsAsAgent: true };
}

/** A5: one endpoint per service. The first served service takes the target's port, each further one the next free port. */
function servicePorts(
	resolved: PlanTargetService[],
	planned: readonly PlannedService[],
	endpoint: PlanTarget["endpoint"],
	device: PlanDevice | undefined,
): PlanTargetService[] {
	const first = endpoint.port;
	if (first === null) return resolved;
	const hosted = new Set(
		planned.filter((service) => service.hosted).map((service) => service.key),
	);
	const own = resolved.map((service) => service.serviceId);
	let last: number | undefined;
	return resolved.map((service) => {
		if (!hosted.has(service.key) || !service.events.length) return service;
		if (last === undefined) {
			last = first;
			return service;
		}
		last = nextFreePort(last + 1, device, own);
		return { ...service, port: last };
	});
}

function planTarget(
	draft: DeployDraft,
	target: DeployTargetDraft,
	services: PlannedService[],
	facts: PlanFacts,
): PlanTarget {
	const device = facts.devices[target.deviceId];
	// An id that had to change also stays clear of the ids this plan already uses on the device.
	const planned = new Set<string>();
	const resolved = services.map((service) => {
		const value = targetService(service, target, device, draft, planned);
		planned.add(value.serviceId);
		return value;
	});
	const endpoint = targetEndpoint(draft, target, device, resolved);
	return {
		deviceId: target.deviceId,
		name: device?.name ?? target.deviceId,
		gate: device?.gate ?? null,
		locked: device?.locked ?? true,
		services: servicePorts(resolved, services, endpoint, device),
		endpoint,
		...targetResources(target.over.isolation ?? draft.isolation, device),
	};
}

/** Derives services and per-device plans; the same function serves the wizard and Update everywhere. */
export function resolvePlan(draft: DeployDraft, facts: PlanFacts): DeployPlan {
	const services = planServices(draft, facts.app);
	return {
		draft,
		app: facts.app,
		mode: facts.app ? appMode(facts.app.visibility) : null,
		services,
		targets: draft.targets.map((target) =>
			planTarget(draft, target, services, facts),
		),
	};
}

/* Approvals (APP §3.10). */

/** The values whose condition holds, in order. */
const flagged = <T>(rows: readonly (readonly [T, boolean])[]): T[] =>
	rows.filter(([, on]) => on).map(([value]) => value);

const inRange = (value: number, min: number, max: number) =>
	Number.isSafeInteger(value) && value >= min && value <= max;

interface ApprovalContext {
	appId: string | null;
	now: number;
	serviceMaxInstances: number;
	isAppOwner?: boolean;
	spending?: SpendingDraft | null;
}

const modelIssues = (approval: ApprovalDraft): ApprovalIssue[] => {
	const { models } = approval;
	const invalid =
		models.length > 64 ||
		new Set(models).size !== models.length ||
		models.some((id) => !SERVICE_ID.test(id));
	return [
		...(invalid ? [{ code: "models_invalid", field: "models" } as const] : []),
		...(!models.length && approval.files === "none"
			? [{ code: "nothing_approved", field: "models" } as const]
			: []),
	];
};

const fileIssues = (
	approval: ApprovalDraft,
	context: ApprovalContext,
): ApprovalIssue[] => {
	if (approval.files === "none") return [];
	const issues: ApprovalIssue[] = [];
	if (!context.appId) issues.push({ code: "files_need_app", field: "files" });
	if (context.isAppOwner === false)
		issues.push({ code: "files_need_owner", field: "files" });
	if (!approval.ownerConsent)
		issues.push({ code: "files_need_consent", field: "files" });
	return issues;
};

const limitIssues = (
	approval: ApprovalDraft,
	context: ApprovalContext,
): ApprovalIssue[] => {
	const issues: ApprovalIssue[] = [];
	const { maxInstances, expiresAt } = approval;
	if (!Number.isInteger(maxInstances) || maxInstances < 1 || maxInstances > 100)
		issues.push({ code: "instances_range", field: "maxInstances" });
	else if (maxInstances < context.serviceMaxInstances)
		issues.push({
			code: "instances_below_service",
			field: "maxInstances",
			params: { count: context.serviceMaxInstances },
		});
	if (expiresAt <= context.now)
		issues.push({ code: "expiry_past", field: "expiresAt" });
	else if (expiresAt > context.now + MAX_APPROVAL_SECONDS)
		issues.push({ code: "expiry_too_far", field: "expiresAt" });
	return issues;
};

const spendingIssues = (
	approval: ApprovalDraft,
	context: ApprovalContext,
): ApprovalIssue[] => {
	const { spending } = context;
	// An approval with models can't be bound to a service without a spending limit (APP §3.10).
	if (spending === null && approval.models.length > 0)
		return [{ code: "spending_required", field: "spending" }];
	if (!spending) return [];
	const failing = flagged<ApprovalIssueCode>([
		["spending_without_models", !approval.models.length],
		["spending_range", !inRange(spending.limitMicros, 1, MAX_BILLING_MICROS)],
		[
			"spending_expiry",
			!inRange(spending.expiresAt, context.now + 1, approval.expiresAt),
		],
		["spending_consent", !spending.consent],
	]);
	return failing.map((code) => ({ code, field: "spending" }));
};

/** The hub's approval rules, checked before anything is sent (`device-resources.ts`, `api/inst`). */
export function checkApprovalDraft(
	approval: ApprovalDraft,
	context: ApprovalContext,
): ApprovalIssue[] {
	return [
		...modelIssues(approval),
		...fileIssues(approval, context),
		...limitIssues(approval, context),
		...spendingIssues(approval, context),
	];
}

/** The hub request for one service on one device. */
export function approvalRequest(
	approval: ApprovalDraft,
	identity: {
		placementId: string;
		deploymentId: string;
		projectId: string;
		appId: string | null;
	},
): CreateResourceGrant {
	return {
		placement_id: identity.placementId,
		deployment_id: identity.deploymentId,
		project_id: identity.projectId,
		app_id: identity.appId,
		...(approval.files === "none" ? {} : { online_access: approval.files }),
		model_ids: approval.models,
		max_instances: approval.maxInstances,
		expires_at: approval.expiresAt,
	};
}

/* Checks (APP §3.5–§3.12): codes per step and per device, never sentences. */

type Issues = PlanIssue[];

type IssueExtra = Omit<PlanIssue, "code" | "step" | "severity"> & {
	severity?: PlanIssue["severity"];
};

const issue = (
	code: PlanIssueCode,
	step: DeployStepId,
	extra: IssueExtra = {},
): PlanIssue => ({ code, step, severity: "error", ...extra });

const ownTarget = (plan: DeployPlan, deviceId: string) =>
	plan.draft.targets.find((value) => value.deviceId === deviceId);

const serviceIdIssues = (
	plan: DeployPlan,
	service: PlannedService,
): PlanIssue[] => {
	const ids = plan.services.map((value) => value.id);
	const serviceKey = service.key;
	const invalid = !isServiceId(service.id);
	return flagged([
		[
			issue("too_many_events", "what", { serviceKey }),
			service.events.length > MAX_EVENTS_PER_SERVICE,
		],
		[issue("service_id_invalid", "what", { serviceKey }), invalid],
		[
			issue("service_id_twice", "what", { serviceKey }),
			!invalid && ids.indexOf(service.id) !== ids.lastIndexOf(service.id),
		],
	]);
};

function checkWhat(plan: DeployPlan, facts: PlanFacts, issues: Issues) {
	issues.push(
		...flagged([
			[issue("app_missing", "what"), !plan.app],
			[
				issue("no_events", "what"),
				!plan.services.some((service) => service.events.length),
			],
		]),
	);
	for (const service of plan.services)
		issues.push(...serviceIdIssues(plan, service));
	if (plan.mode === "offline" && facts.platform === "web")
		issues.push(issue("local_only_web", "how"));
}

/** Another service of the plan would write the same service on this device. */
const sharesId = (target: PlanTarget, service: PlanTargetService) =>
	target.services.some(
		(other) => other !== service && other.serviceId === service.serviceId,
	);

const targetServiceIssues = (
	plan: DeployPlan,
	target: PlanTarget,
	service: PlanTargetService,
	reserved: readonly string[],
): PlanIssue[] => {
	const at = { deviceId: target.deviceId, serviceKey: service.key };
	const params = { service: service.serviceId };
	return flagged([
		[
			issue("service_id_reserved", "where", { ...at, params }),
			service.kind === "new" && reserved.includes(service.serviceId),
		],
		[
			issue("service_id_twice", "where", { ...at, params }),
			sharesId(target, service),
		],
		[
			// An update entry never creates a service: the caller picks the one to update per device.
			issue("update_needs_service", "where", at),
			service.kind === "new" && plan.draft.entry === "update",
		],
		[
			issue("removed_events", "what", {
				...at,
				params: { count: service.removedEvents.length },
			}),
			service.removedEvents.length > 0 && !plan.draft.acceptRemovedEvents,
		],
	]);
};

function checkTarget(
	plan: DeployPlan,
	target: PlanTarget,
	facts: PlanFacts,
	issues: Issues,
) {
	const deviceId = target.deviceId;
	issues.push(
		...flagged([
			[issue("target_gated", "where", { deviceId }), target.gate !== null],
			[issue("target_locked", "where", { deviceId }), target.locked],
			[
				issue("target_no_events", "where", { deviceId }),
				!target.services.some((service) => service.events.length),
			],
		]),
	);
	const reserved = facts.devices[deviceId]?.reservedIds ?? [];
	for (const service of target.services)
		issues.push(...targetServiceIssues(plan, target, service, reserved));
}

function serviceVariables(plan: DeployPlan, service: PlannedService) {
	const app = plan.app;
	if (!app) return [];
	try {
		return mergeVariables(
			service.events.map((eventId) => [...eventVariables(app, eventId)]),
		);
	} catch {
		return [];
	}
}

function valueIssue(variable: DeploymentVariable, text: string) {
	try {
		const value = variableValue(variable, text);
		if (!variable.secret) return null;
		const bytes = new TextEncoder().encode(JSON.stringify(value)).length;
		return bytes >= 1 && bytes <= 4096 ? null : "secret_size";
	} catch {
		return "variable_invalid";
	}
}

function targetValues(
	draft: DeployDraft,
	target: DeployTargetDraft | undefined,
): Record<string, string> {
	return {
		...draft.vars,
		...target?.over.vars,
		...(draft.secretsMode === "same" ? draft.secrets : {}),
		...target?.over.secrets,
	};
}

function checkSettings(plan: DeployPlan, issues: Issues) {
	const variables = [
		...new Map(
			plan.services
				.flatMap((service) => serviceVariables(plan, service))
				.map((variable) => [variable.id, variable]),
		).values(),
	];
	for (const target of plan.draft.targets) {
		const values = targetValues(plan.draft, target);
		for (const variable of variables) {
			const text = values[variable.id];
			if (text === undefined || (variable.secret && text === "")) continue;
			const code = valueIssue(variable, text);
			if (code)
				issues.push(
					issue(code, "settings", {
						deviceId: target.deviceId,
						params: { variable: variable.name },
					}),
				);
		}
	}
}

function isLoopback(host: string) {
	return host === "::1" || host.startsWith("127.");
}

function tokenFor(draft: DeployDraft, target: DeployTargetDraft) {
	return target.over.token ?? draft.endpoint.tokenValue;
}

function checkToken(draft: DeployDraft, target: PlanTarget, issues: Issues) {
	const deviceId = target.deviceId;
	const own = draft.targets.find((value) => value.deviceId === deviceId);
	const creates = target.services.some((service) => service.kind === "new");
	const mode = draft.endpoint.token;
	const token = own ? tokenFor(draft, own) : "";
	const invalid =
		mode === "keep" ? creates : mode !== "per_device" && !TOKEN.test(token);
	if (invalid) issues.push(issue("token_invalid", "endpoint", { deviceId }));
}

function validHost(host: string) {
	return z.string().ip().safeParse(host).success;
}

function validPort(port: number) {
	return Number.isInteger(port) && port >= 1 && port <= 65535;
}

/** The target's port and every port a further served service of the plan was given. */
function plannedPorts(target: PlanTarget): number[] {
	return [
		target.endpoint.port,
		...target.services.map((service) => service.port),
	].filter((port): port is number => typeof port === "number");
}

function portIssues(
	target: PlanTarget,
	device: PlanDevice | undefined,
): PlanIssue[] {
	const deviceId = target.deviceId;
	const ports = plannedPorts(target);
	if (ports.some((port) => !validPort(port)))
		return [issue("port_invalid", "endpoint", { deviceId })];
	const own = new Set(target.services.map((service) => service.serviceId));
	return (device?.portsInUse ?? [])
		.filter(
			(row) =>
				ports.includes(row.port) && !(row.serviceId && own.has(row.serviceId)),
		)
		.map((row) =>
			issue("port_in_use", "endpoint", {
				deviceId,
				params: {
					port: row.port,
					...(row.serviceId ? { service: row.serviceId } : {}),
				},
			}),
		);
}

function checkAddress(
	target: PlanTarget,
	device: PlanDevice | undefined,
	issues: Issues,
) {
	const { host } = target.endpoint;
	if (host !== null && !validHost(host))
		issues.push(
			issue("host_invalid", "endpoint", { deviceId: target.deviceId }),
		);
	issues.push(...portIssues(target, device));
}

const portMoved = (deviceId: string, params: CopyParams): PlanException => ({
	code: "port_moved",
	step: "endpoint",
	deviceId,
	tone: "info",
	params,
});

/** The target's port when it had to move, and every further service that took the next free one. */
function portExceptions(target: PlanTarget): PlanException[] {
	const { port, portMovedFrom } = target.endpoint;
	if (port === null) return [];
	const first =
		portMovedFrom === undefined
			? []
			: [portMoved(target.deviceId, { from: portMovedFrom, to: port })];
	const further = target.services.flatMap((service) =>
		service.port === undefined
			? []
			: [
					portMoved(target.deviceId, {
						from: port,
						to: service.port,
						service: service.serviceId,
					}),
				],
	);
	return [...first, ...further];
}

function servesUnencrypted(target: PlanTarget) {
	const { host, certificateId } = target.endpoint;
	return (
		certificateId === null &&
		host !== null &&
		validHost(host) &&
		!isLoopback(host)
	);
}

function checkEndpoint(
	plan: DeployPlan,
	target: PlanTarget,
	facts: PlanFacts,
	issues: Issues,
	exceptions: PlanException[],
) {
	const deviceId = target.deviceId;
	checkAddress(target, facts.devices[deviceId], issues);
	exceptions.push(...portExceptions(target));
	checkToken(plan.draft, target, issues);
	if (!servesUnencrypted(target)) return;
	exceptions.push({
		code: "no_certificate",
		step: "endpoint",
		deviceId,
		tone: "warning",
	});
	if (!ownTarget(plan, deviceId)?.over.allowUnencrypted)
		issues.push(issue("unencrypted", "endpoint", { deviceId }));
}

const isolationIssues = (
	target: PlanTarget,
	profile: IsolationProfile | undefined,
	policy: HostIsolationMode | undefined,
): PlanIssue[] => {
	const deviceId = target.deviceId;
	return flagged([
		[
			issue("isolation_unavailable", "endpoint", { deviceId }),
			profile === "linux_sandbox" && policy === "none",
		],
		[
			issue("isolation_required", "endpoint", { deviceId }),
			target.runsAsAgent && policy === "required",
		],
	]);
};

function checkIsolation(
	plan: DeployPlan,
	target: PlanTarget,
	facts: PlanFacts,
	issues: Issues,
	exceptions: PlanException[],
) {
	const deviceId = target.deviceId;
	const own = ownTarget(plan, deviceId);
	const profile = (own?.over.isolation ?? plan.draft.isolation)?.profile;
	issues.push(
		...isolationIssues(target, profile, facts.devices[deviceId]?.isolation),
	);
	if (!target.runsAsAgent || target.locked) return;
	exceptions.push({
		code: "runs_as_agent",
		step: "endpoint",
		deviceId,
		tone: "warning",
	});
	if (!own?.over.trustAgent)
		issues.push(issue("agent_trust", "endpoint", { deviceId }));
}

function checkLimits(plan: DeployPlan, issues: Issues) {
	const { maxInstances, writes } = plan.draft;
	if (!Number.isInteger(maxInstances) || maxInstances < 1 || maxInstances > 32)
		issues.push(issue("service_instances_range", "endpoint"));
	if (!writes) return;
	if (plan.mode !== "online")
		issues.push(issue("writes_offline", "access_cost"));
	else if (!offlineWritesSchema.safeParse(writes).success)
		issues.push(issue("writes_invalid", "access_cost"));
}

/** New online services always get an approval; offline copies only for hosted models (APP §3.11). */
function wantsApproval(plan: DeployPlan) {
	return plan.mode === "online" || plan.draft.approval.models.length > 0;
}

function checkAccess(plan: DeployPlan, facts: PlanFacts, issues: Issues) {
	const creates = plan.targets.some((target) =>
		target.services.some((service) => service.kind === "new"),
	);
	const { approval, spending, appId } = plan.draft;
	if (!creates || !wantsApproval(plan)) return;
	const step = plan.mode === "online" ? "access_cost" : "copy_upload";
	const found = checkApprovalDraft(approval, {
		appId: plan.mode === "online" ? appId : null,
		now: facts.now,
		serviceMaxInstances: Math.max(
			1,
			...plan.services.map((service) => service.maxInstances),
		),
		isAppOwner: facts.isAppOwner,
		spending,
	});
	for (const row of found)
		issues.push(issue(`approval.${row.code}`, step, { params: row.params }));
}

function targetExceptions(target: PlanTarget): PlanException[] {
	const deviceId = target.deviceId;
	return target.services.flatMap((service) => [
		...service.leftOut.map(
			(row): PlanException => ({
				code: row.why === "refuse" ? "left_out_refuse" : "left_out_duplicate",
				step: "where",
				deviceId,
				tone: "paused",
				params: {
					event: row.eventId,
					...(row.why === "refuse"
						? { reason: row.detail }
						: { service: row.serviceId }),
				},
			}),
		),
		...(service.renamedFrom
			? [
					{
						code: "renamed",
						step: "where",
						deviceId,
						tone: "info",
						params: { from: service.renamedFrom, to: service.serviceId },
					} satisfies PlanException,
				]
			: []),
	]);
}

function serves(plan: DeployPlan, target: PlanTarget) {
	const hosted = new Set(
		plan.services.filter((service) => service.hosted).map((s) => s.key),
	);
	return target.services.some(
		(service) => hosted.has(service.key) && service.events.length > 0,
	);
}

/** Per-step errors, per-device exceptions and the first blocking error. */
export function checkPlan(plan: DeployPlan, facts: PlanFacts): PlanCheck {
	const issues: Issues = [];
	const exceptions: PlanException[] = [];
	checkWhat(plan, facts, issues);
	if (!plan.targets.length) issues.push(issue("no_targets", "where"));
	for (const target of plan.targets) {
		checkTarget(plan, target, facts, issues);
		exceptions.push(...targetExceptions(target));
		if (serves(plan, target))
			checkEndpoint(plan, target, facts, issues, exceptions);
		checkIsolation(plan, target, facts, issues, exceptions);
	}
	checkSettings(plan, issues);
	checkLimits(plan, issues);
	checkAccess(plan, facts, issues);
	const ordered = [...issues].sort(
		(a, b) => STEP_ORDER.indexOf(a.step) - STEP_ORDER.indexOf(b.step),
	);
	const firstBlocking =
		ordered.find((value) => value.severity === "error") ?? null;
	return { ok: !firstBlocking, issues: ordered, exceptions, firstBlocking };
}

/* Wiring (oracle: today's `createDeploymentPlan`). */

export interface WireFacts {
	installed: InstalledProject;
	existing?: PlacementConfiguration;
	/** The catalogue the device verifies: approved bundle (online) or discovery (offline). */
	events: readonly DeploymentEvent[];
	variables: Readonly<Record<string, readonly DeploymentVariable[]>>;
	previousVariables?: DeploymentVariable[];
	/** This device's token: generated, shared or typed; "" keeps the current one. */
	serviceToken: string;
	resourceGrant?: Record<string, unknown>;
	canManageCertificates: boolean;
}

function wireTarget(
	plan: DeployPlan,
	deviceId: string,
	serviceKey: string,
): {
	target: PlanTarget;
	service: PlanTargetService;
	draft: DeployTargetDraft;
} {
	const target = plan.targets.find((value) => value.deviceId === deviceId);
	const service = target?.services.find((value) => value.key === serviceKey);
	const draft = plan.draft.targets.find((value) => value.deviceId === deviceId);
	if (!target || !service || !draft)
		throw new Error(
			`The deploy plan has no service ${serviceKey} on device ${deviceId}. Rebuild the plan before wiring it.`,
		);
	return { target, service, draft };
}

function wireOverrides(
	plan: DeployPlan,
	target: DeployTargetDraft,
	variables: readonly DeploymentVariable[],
	existing: PlacementConfiguration | undefined,
): Record<string, string> {
	const values = targetValues(plan.draft, target);
	return Object.fromEntries(
		Object.entries(values).filter(([id, value]) => {
			const definition = variables.find((variable) => variable.id === id);
			return (
				definition &&
				(!existing || plan.draft.edited.includes(id)) &&
				(value !== "" || !definition.secret)
			);
		}),
	);
}

function wireCertificate(
	target: PlanTarget,
	facts: WireFacts,
): string | null | undefined {
	if (!facts.canManageCertificates) return undefined;
	const chosen = target.endpoint.certificateId;
	return chosen === undefined
		? (facts.existing?.config.tls_certificate_id ?? null)
		: chosen;
}

/** The `createDeploymentPlan` input for one service on one device. */
export function wirePlan(
	plan: DeployPlan,
	at: { deviceId: string; serviceKey: string },
	facts: WireFacts,
): DeploymentPlanInput {
	const { target, service, draft } = wireTarget(
		plan,
		at.deviceId,
		at.serviceKey,
	);
	const { existing, installed } = facts;
	const events = facts.events.filter((event) =>
		service.events.includes(event.id),
	);
	const variables = mergeVariables(
		events.map((event) => [...(facts.variables[event.id] ?? [])]),
	);
	const hosting = existing?.config.hosting;
	const planned = plan.services.find((value) => value.key === service.key);
	return {
		installed,
		existing,
		healthChecked:
			plan.draft.strategy !== "quick" &&
			canCheckDeploymentStartup(installed, existing, events),
		removeOverrides: draft.over.removeOverrides ?? [],
		placement: service.serviceId,
		deployment: existing?.deployment_id ?? plan.draft.deploymentId,
		events,
		variables,
		previousVariables: facts.previousVariables ?? [],
		overrides: wireOverrides(plan, draft, variables, existing),
		host: target.endpoint.host ?? hosting?.host ?? "127.0.0.1",
		port: service.port ?? target.endpoint.port ?? hosting?.port ?? 8080,
		replicas: existing?.config.max_replicas ?? planned?.maxInstances ?? 1,
		serviceToken: facts.serviceToken,
		tlsCertificateId: wireCertificate(target, facts),
		offlineWrites: plan.draft.writes,
		resourceLimits: target.resources,
		resourceGrant: facts.resourceGrant,
	};
}

/* Phases (APP §3.13; today's `phasesFor`). */

interface PhaseOptions {
	secrets: boolean;
	safe: boolean;
}

function updatePhases(plan: DeployPlan, options: PhaseOptions): DeployPhase[] {
	const ship: DeployPhase[] =
		plan.draft.version === "keep" ? [] : ["upload", "install"];
	const secrets: DeployPhase[] = options.secrets ? ["secrets"] : [];
	// The device stages a safe update first; its new secrets are written into the staged update.
	return options.safe
		? [...ship, "prepare_update", ...secrets, "check_new", "switch"]
		: [...ship, "stop", ...secrets, "start"];
}

function accessPhases(plan: DeployPlan): DeployPhase[] {
	if (!wantsApproval(plan)) return [];
	return plan.draft.spending ? ["approve", "spending"] : ["approve"];
}

export function planPhases(
	plan: DeployPlan,
	service: PlanTargetService,
	options: PhaseOptions,
): DeployPhase[] {
	if (service.kind !== "new") return updatePhases(plan, options);
	return [
		...accessPhases(plan),
		"upload",
		...(plan.mode === "offline" ? (["check_events"] as const) : []),
		"install",
		"create",
		...(options.secrets ? (["secrets"] as const) : []),
		...(plan.draft.start ? (["start"] as const) : []),
	];
}

/* Config diff (N3 › Edit settings, deploy Review, Update everywhere). */

export type DiffField =
	| "app_version"
	| "definitions"
	| "event"
	| "variable"
	| "secret"
	| "endpoint_host"
	| "endpoint_port"
	| "token"
	| "certificate"
	| "instances"
	| "cloud_access"
	| "spending"
	| "write_buffering"
	| "isolation"
	| "packages";

/** Raw config values for the caller to format; secret rows never carry values. */
export interface ConfigDiffRow {
	kind: "added" | "changed" | "removed";
	field: DiffField;
	key?: string;
	before?: unknown;
	after?: unknown;
}

type Config = Readonly<Record<string, unknown>>;

function record(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

function same(left: unknown, right: unknown) {
	return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

const scalarRow = (
	field: DiffField,
	before: unknown,
	after: unknown,
): ConfigDiffRow[] => {
	if (same(before, after)) return [];
	const kind = before == null ? "added" : after == null ? "removed" : "changed";
	return [
		{
			kind,
			field,
			...(before == null ? {} : { before }),
			...(after == null ? {} : { after }),
		},
	];
};

const keyedRows = (
	field: DiffField,
	before: Record<string, unknown>,
	after: Record<string, unknown>,
	withValues: boolean,
): ConfigDiffRow[] => {
	const keys = [
		...new Set([...Object.keys(before), ...Object.keys(after)]),
	].sort();
	return keys.flatMap((key) =>
		scalarRow(field, before[key], after[key]).map((row) =>
			withValues ? { ...row, key } : { kind: row.kind, field, key },
		),
	);
};

function eventMap(config: Config): Record<string, unknown> {
	const events = Array.isArray(config.events) ? config.events : [];
	return Object.fromEntries(
		events.map((event) => {
			const { event_id, ...pins } = record(event);
			return [String(event_id), pins];
		}),
	);
}

function packagePins(config: Config) {
	const pins = {
		bit_pins: Array.isArray(config.bit_pins) ? config.bit_pins : [],
		package_pins: Array.isArray(config.package_pins) ? config.package_pins : [],
	};
	return pins.bit_pins.length || pins.package_pins.length ? pins : null;
}

function grantId(config: Config, key: string) {
	return record(config.resource_grant)[key];
}

function identityRows(old: Config, after: Config) {
	return [
		...scalarRow("app_version", old.revision, after.revision),
		...scalarRow(
			"definitions",
			old.online_metadata_sha256,
			after.online_metadata_sha256,
		),
		...keyedRows("event", eventMap(old), eventMap(after), true),
		...keyedRows(
			"variable",
			record(old.variables),
			record(after.variables),
			true,
		),
		...keyedRows(
			"secret",
			record(old.secret_overrides),
			record(after.secret_overrides),
			false,
		),
	];
}

function endpointRows(old: Config, after: Config) {
	const before = record(old.hosting);
	const next = record(after.hosting);
	return [
		...scalarRow("endpoint_host", before.host, next.host),
		...scalarRow("endpoint_port", before.port, next.port),
		...scalarRow("token", before.auth_secret, next.auth_secret).map(
			(row): ConfigDiffRow => ({ kind: row.kind, field: "token" }),
		),
		...scalarRow(
			"certificate",
			old.tls_certificate_id,
			after.tls_certificate_id,
		),
	];
}

function runtimeRows(old: Config, after: Config) {
	return [
		...scalarRow("instances", old.max_replicas, after.max_replicas),
		...scalarRow(
			"cloud_access",
			grantId(old, "grant_id"),
			grantId(after, "grant_id"),
		),
		...scalarRow(
			"spending",
			grantId(old, "billing_grant_id"),
			grantId(after, "billing_grant_id"),
		),
		...scalarRow("write_buffering", old.offline_writes, after.offline_writes),
		...scalarRow("isolation", old.resources, after.resources),
		...scalarRow("packages", packagePins(old), packagePins(after)),
	];
}

/** One config diff for every surface; secret values and references never leave it. */
export function diffPlacementConfig(
	before: Config | null | undefined,
	after: Config,
): ConfigDiffRow[] {
	const old = before ?? {};
	return [
		...identityRows(old, after),
		...endpointRows(old, after),
		...runtimeRows(old, after),
	];
}
