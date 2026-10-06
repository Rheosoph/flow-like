import { z } from "zod";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IEventState } from "../../state/backend-state/event-state";
import { CRON_EVENT_TYPE, type IScheduleConfig } from "../schedule-config";
import { ONCE_MAX_AT, ONCE_MIN_AT } from "../schedule-instant";
import type { IBoard } from "../schema/flow/board";
import type { IEvent } from "../schema/flow/event";
import { parseUint8ArrayToJson } from "../uint8";
import {
	type ProjectArtifactAssets,
	assertOneVersionPerPackage,
} from "./artifacts";
import {
	type BotFacts,
	type DeviceBotResult,
	botProvider,
	botTokenEventId,
	botTokenKey,
	botTokenProblem,
	botTokenVariable,
	deviceBot,
} from "./bot-config";
import {
	type DeviceRouteResult,
	type EventRoute,
	MAX_ROUTE_PATH_BYTES,
	ROUTE_EVENT_TYPES,
	ROUTE_METHODS,
	ROUTE_PROBLEMS,
	deviceRoute,
} from "./event-route";
import type { AgentFeature, AgentFeatures } from "./model/types";
import { readOfflineQueues } from "./offline-queue";
import {
	type DeviceSchedule,
	type DeviceScheduleResult,
	MAX_SCHEDULE_EXPRESSION,
	type OnceSchedule,
	SCHEDULE_PROBLEMS,
	type ScheduleDetail,
	deviceSchedule,
} from "./schedule";
import type { ManagementCall } from "./telemetry";
import { tunnelServicesSchema } from "./tunnel-services";
import {
	type ManagementRejection,
	type ManagementResponse,
	managementRejection,
} from "./types";

function assertBoundedJson(value: unknown, maxBytes: number): void {
	let remaining = 8192;
	const active = new Set<object>();
	function visit(item: unknown, depth: number) {
		if (--remaining < 0 || depth > 32)
			throw new Error(
				"The deployment configuration exceeds its structure limit.",
			);
		if (item === null || typeof item === "string" || typeof item === "boolean")
			return;
		if (typeof item === "number" && Number.isFinite(item)) return;
		if (typeof item !== "object" || active.has(item))
			throw new Error("The deployment configuration must contain JSON values.");
		if (
			!Array.isArray(item) &&
			Object.getPrototypeOf(item) !== Object.prototype &&
			Object.getPrototypeOf(item) !== null
		)
			throw new Error(
				"The deployment configuration must contain JSON objects.",
			);
		active.add(item);
		for (const child of Object.values(item)) visit(child, depth + 1);
		active.delete(item);
	}
	visit(value, 0);
	if (new TextEncoder().encode(JSON.stringify(value)).length > maxBytes)
		throw new Error(
			"The deployment configuration exceeds the management message limit.",
		);
}

function equivalent(left: unknown, right: unknown): boolean {
	if (Object.is(left, right)) return true;
	if (
		left === null ||
		right === null ||
		typeof left !== "object" ||
		typeof right !== "object"
	)
		return false;
	if (Array.isArray(left) !== Array.isArray(right)) return false;
	const a = Object.keys(left).sort();
	const b = Object.keys(right).sort();
	return (
		a.length === b.length &&
		a.every(
			(key, index) =>
				key === b[index] &&
				equivalent(
					(left as Record<string, unknown>)[key],
					(right as Record<string, unknown>)[key],
				),
		)
	);
}

const identifier = z
	.string()
	.regex(/^[A-Za-z0-9_.-]{1,128}$/)
	.refine((v) => v !== "." && v !== "..");
const version = z.tuple([
	z.number().int().min(0).max(4294967294),
	z.number().int().min(0).max(4294967294),
	z.number().int().min(0).max(4294967294),
]);
/**
 * How an event runs on a device. `on_demand`: a person starts it (quick
 * action, form). `bot`: it stays connected to Telegram or Discord.
 */
export const EVENT_KINDS = [
	"served",
	"own_server",
	"background",
	"scheduled",
	"on_demand",
	"bot",
] as const;
export type EventKind = (typeof EVENT_KINDS)[number];
/** The codes a device's discovery row may carry beside `eligible: false`. */
export const DEVICE_INELIGIBLE_CODES = [
	...SCHEDULE_PROBLEMS,
	...ROUTE_PROBLEMS,
	"bot_invalid",
] as const;
/** A fact newer agents add: a value this client does not know is dropped, never fatal. */
const optionalFact = <T extends z.ZodTypeAny>(schema: T) =>
	schema.nullish().catch(undefined);
const discoveredEventSchema = z.object({
	id: identifier,
	name: z.string().max(480),
	event_type: z.string().max(128),
	event_version: version.nullable(),
	board_version: version.nullable(),
	hosted: z.boolean(),
	readiness_kind: z.enum(["listener", "explicit", "unsupported"]).optional(),
	rollout_supported: z.boolean().optional(),
	eligible: z.boolean(),
	ineligible_reason: z.string().max(480).optional(),
	/** The device's own sentence for an event it can't run. */
	readiness_error: optionalFact(z.string().max(1024)),
	kind: optionalFact(z.enum(EVENT_KINDS)),
	/** A `cron` event whose schedule the device can run; the zone is the effective one. */
	schedule: optionalFact(
		z.object({
			expression: z.string().min(1).max(MAX_SCHEDULE_EXPRESSION),
			timezone: z.string().min(1).max(64),
		}),
	),
	/** An `http` or `api` event without a Page whose route the device serves. */
	route: optionalFact(
		z.object({
			method: z.enum(ROUTE_METHODS),
			path: z.string().min(1).max(MAX_ROUTE_PATH_BYTES),
		}),
	),
	/** A one-time schedule the device can run; `at` in unix seconds. Such a row has no `schedule`. */
	once: optionalFact(
		z.object({
			date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/),
			time: z.string().regex(/^\d{2}:\d{2}$/),
			at: z.number().int().min(ONCE_MIN_AT).max(ONCE_MAX_AT),
			timezone: z.string().min(1).max(64),
		}),
	),
	ineligible_code: optionalFact(z.enum(DEVICE_INELIGIBLE_CODES)),
});
/** A row whose `kind` this client does not know can't run here, whatever the device says. */
const eventSchema = z.preprocess((row) => {
	if (!row || typeof row !== "object" || Array.isArray(row)) return row;
	const { kind } = row as { kind?: unknown };
	return typeof kind === "string" &&
		!(EVENT_KINDS as readonly string[]).includes(kind)
		? { ...row, kind: undefined, eligible: false }
		: row;
}, discoveredEventSchema);
const variableSchema = z.object({
	id: identifier,
	name: z.string().max(480),
	data_type: z.string().max(64),
	value_type: z.string().max(64),
	secret: z.boolean(),
});
const hostingSchema = z
	.object({
		host: z.string().ip(),
		port: z.number().int().min(1).max(65535),
		max_in_flight: z.number().int().min(1).max(1024),
		request_timeout_secs: z.number().int().min(1).max(3600),
		authentication: z.enum(["token", "none"]).optional(),
		auth_secret: identifier.nullish(),
	})
	.passthrough()
	.refine((hosting) =>
		hosting.authentication === "none"
			? hosting.auth_secret == null
			: hosting.auth_secret != null,
	);
const resourceGrantSchema = z
	.object({
		grant_id: identifier,
		authz_version: z.number().int().positive().safe(),
		billing_grant_id: identifier.nullish(),
		billing_authz_version: z.number().int().positive().safe().nullish(),
	})
	.passthrough()
	.refine(
		(grant) =>
			(grant.billing_grant_id == null) ===
			(grant.billing_authz_version == null),
	);
const relativeResourcePath = (empty: boolean) =>
	z
		.string()
		.refine(
			(value) =>
				(empty && value === "") ||
				(value.length > 0 &&
					new TextEncoder().encode(value).length <= 1024 &&
					!/[\\%\0]/.test(value) &&
					value
						.split("/")
						.every((part) => part !== "" && part !== "." && part !== "..")),
			"Use a relative path without empty segments, parent traversal or percent escapes.",
		);

export const offlineWritesSchema = z
	.object({
		tables: z
			.array(
				z
					.object({
						purpose: z.enum(["storage", "user"]),
						database: z.literal("db"),
						table: z.string().regex(/^[A-Za-z0-9_-]{1,128}$/),
						primary_key: z.string().regex(/^[A-Za-z0-9_]{1,128}$/),
					})
					.strict(),
			)
			.default([]),
		files: z
			.array(
				z
					.object({
						purpose: z.enum(["files", "storage", "user", "temporary"]),
						prefix: relativeResourcePath(false).refine(
							(value) =>
								!value.split("/").some((part) => part.endsWith(".lance")),
							"Choose a directory outside Lance tables.",
						),
					})
					.strict()
					.refine(
						(file) =>
							!["storage", "user"].includes(file.purpose) ||
							file.prefix.split("/")[0] !== "db",
						"Lance database directories cannot be buffered as files.",
					),
			)
			.default([]),
		max_queue_bytes: z
			.number()
			.int()
			.min(1048576)
			.max(64 * 1024 ** 3)
			.default(256 * 1048576),
		max_operations: z.number().int().min(1).max(100000).default(10000),
		max_age_seconds: z
			.number()
			.int()
			.min(60)
			.max(30 * 86400)
			.default(7 * 86400),
		max_mirror_bytes: z
			.number()
			.int()
			.min(1048576)
			.max(1024 ** 4)
			.default(2 * 1024 ** 3),
	})
	.strict()
	.superRefine((value, context) => {
		if (
			value.tables.length + value.files.length < 1 ||
			value.tables.length + value.files.length > 64
		)
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "Select between 1 and 64 tables or file directories.",
			});
		const tables = value.tables.map((table) =>
			JSON.stringify([table.purpose, table.database, table.table]),
		);
		if (new Set(tables).size !== tables.length)
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "Each buffered table may be selected only once.",
			});
	});
export type OfflineWritesConfig = z.infer<typeof offlineWritesSchema>;

export const placementResourcesSchema = z
	.object({
		profile: z.enum(["trusted_process", "linux_sandbox"]),
		cpu_millis: z.number().int().min(1).max(1024000).nullish(),
		memory_bytes: z
			.number()
			.int()
			.min(64 * 1024 ** 2)
			.max(16 * 1024 ** 4)
			.nullish(),
		max_processes: z.number().int().min(16).max(65536).nullish(),
		disk_bytes: z
			.number()
			.int()
			.min(16 * 1024 ** 2)
			.max(1024 ** 5)
			.nullish(),
	})
	.strict()
	.superRefine((value, context) => {
		const bounds = [
			value.cpu_millis,
			value.memory_bytes,
			value.max_processes,
			value.disk_bytes,
		];
		if (
			value.profile === "linux_sandbox"
				? bounds.some((bound) => bound == null)
				: bounds.some((bound) => bound != null)
		)
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message:
					"Linux isolation requires every resource limit; the trusted process profile enforces none.",
			});
	});
export type PlacementResources = z.infer<typeof placementResourcesSchema>;

// Unknown fields pass through so an update keeps settings this form does not edit.
const placementConfigSchema = z
	.object({
		id: identifier,
		project_id: identifier,
		deployment_id: identifier,
		revision: z
			.string()
			.min(1)
			.max(256)
			.refine((value) => value.trim().length > 0),
		source: z.enum(["offline", "online"]),
		project_path: z.string().min(1).max(4096),
		online_metadata_sha256: z
			.string()
			.regex(/^[a-f0-9]{64}$/)
			.nullish(),
		events: z
			.array(
				z.object({
					event_id: identifier,
					event_version: version,
					board_version: version,
				}),
			)
			.min(1)
			.max(64),
		hosting: hostingSchema.nullish(),
		tls_certificate_id: z.string().uuid().nullish(),
		tunnel_services: tunnelServicesSchema.optional(),
		max_replicas: z.number().int().min(1).max(32),
		variables: z.record(identifier, z.unknown()).default({}),
		secret_overrides: z.record(identifier, identifier).default({}),
		resource_grant: resourceGrantSchema.nullish(),
		offline_writes: offlineWritesSchema.nullish(),
		resources: placementResourcesSchema.nullish(),
	})
	.passthrough()
	.refine((config) => {
		const plain = Object.keys(config.variables);
		const secret = Object.keys(config.secret_overrides);
		return (
			plain.length + secret.length <= 1024 &&
			secret.every((id) => !Object.hasOwn(config.variables, id)) &&
			new Set(config.events.map((event) => event.event_id)).size ===
				config.events.length &&
			(config.source !== "online" || config.resource_grant != null) &&
			(config.source === "online" || config.offline_writes == null) &&
			(config.offline_writes == null || config.max_replicas === 1) &&
			(config.max_replicas === 1 || config.hosting != null)
		);
	});
export type DeploymentEvent = z.infer<typeof eventSchema>;
export type DeploymentVariable = z.infer<typeof variableSchema>;
const unixSeconds = z.number().int().safe();
const revisionNumber = z.number().int().nonnegative().safe();
const replicaCount = z.number().int().min(0).max(32);
// Agents add timeline fields over time; unknown fields pass through instead of failing the read.
export const rolloutSchema = z
	.object({
		rollout_id: identifier,
		placement_id: identifier,
		project_id: identifier,
		state: z.enum([
			"staged",
			"validating",
			"activating",
			"healthy",
			"rolling_back",
			"rolled_back",
			"failed",
			"cancelled",
		]),
		failure_code: z.string().max(128).nullish(),
		active_revision: z.number().int().positive().safe().nullish(),
		active_intent: revisionNumber.nullish(),
		base_revision: revisionNumber.optional(),
		previous_replicas: replicaCount.optional(),
		candidate_replicas: replicaCount.optional(),
		stabilization_seconds: z.number().int().min(0).max(3600).optional(),
		deadline_seconds: z.number().int().min(0).max(86400).optional(),
		created_at: unixSeconds.optional(),
		updated_at: unixSeconds.optional(),
		deadline_at: unixSeconds.nullish(),
		stable_since: unixSeconds.nullish(),
	})
	.passthrough();
export type DeploymentRolloutStatus = z.infer<typeof rolloutSchema>;
export type PlacementConfiguration = {
	placement_id: string;
	project_id: string;
	deployment_id: string;
	config_revision: number;
	config: z.infer<typeof placementConfigSchema>;
	desired_state?: "running" | "stopped";
	rollout_sources?: ("offline" | "online")[];
	rollout?: DeploymentRolloutStatus | null;
};
export class StaleDeploymentRevisionError extends Error {
	constructor(placementId: string, expected: number, current: number) {
		super(
			`Placement ${placementId} changed on the device: configuration revision ${current} replaced revision ${expected}. Reload the placement before updating it.`,
		);
		this.name = "StaleDeploymentRevisionError";
	}
}
export class DeploymentPublicationFailedError extends Error {
	constructor(placementId: string, detail?: string) {
		super(
			`Secret publication failed for placement ${placementId}.${detail ? ` ${detail}` : ""} Reload this existing placement before preparing another update.`,
		);
		this.name = "DeploymentPublicationFailedError";
	}
}
const rejectionHints: Record<string, string> = {
	unauthorized: "Your access to this device does not allow this change.",
	revision_conflict:
		"The placement identity or revision differs on the device. Reload the placement, or choose another placement ID for a new placement.",
	invalid: "Change the configuration before trying again.",
	host_policy:
		"Change the isolation settings to satisfy the device host policy.",
	unsupported:
		"The device agent does not support this setting. Update the agent or choose another setting.",
	limit:
		"A device capacity limit was reached. Free capacity on the device or reduce the request.",
};
/** A definitive device refusal: retrying the same operations cannot succeed. */
export class DeploymentRejectedError extends Error {
	constructor(
		readonly rejection: ManagementRejection,
		action: string,
		hint = Object.hasOwn(rejectionHints, rejection.code)
			? rejectionHints[rejection.code]
			: undefined,
	) {
		super(
			`The device rejected ${action}: ${rejection.error}${hint ? ` ${hint}` : ""}`,
		);
		this.name = "DeploymentRejectedError";
	}
}
function finalRejection(
	response: Pick<ManagementResponse, "state" | "result">,
): ManagementRejection | undefined {
	const rejection = managementRejection(response);
	return rejection && !rejection.retryable ? rejection : undefined;
}
function rejectionDetail(
	response: Pick<ManagementResponse, "state" | "result">,
) {
	const rejection = managementRejection(response);
	return rejection ? ` Device response: ${rejection.error}` : "";
}
export class DeploymentRolloutEndedError extends Error {
	constructor(readonly status: DeploymentRolloutStatus) {
		super(
			status.state === "rolled_back"
				? "The new services did not pass startup checks. The device restored the previous configuration and secret references. Reload the placement to review its status."
				: status.state === "cancelled"
					? "The rollout was cancelled. Reload the placement to review its current state."
					: "The rollout failed. Reload the placement to review its current state before another update.",
		);
		this.name = "DeploymentRolloutEndedError";
	}
}
export class DeploymentReviewRequiredError extends Error {
	constructor(message: string) {
		super(message);
		this.name = "DeploymentReviewRequiredError";
	}
}
export async function readExistingDeployment(
	call: ManagementCall,
	placementId: string,
	projectId: string,
): Promise<PlacementConfiguration> {
	identifier.parse(placementId);
	identifier.parse(projectId);
	const response = await call({
		type: "placement_configuration",
		placement_id: placementId,
	});
	if (response.state !== "completed")
		throw new Error(
			`The device did not return the configuration of placement ${placementId}. Updating it requires deploy access to this placement.${rejectionDetail(response)}`,
		);
	assertBoundedJson(response.result, 16 * 1024);
	const {
		config_revision,
		config,
		deployment_id,
		desired_state,
		rollout_sources,
		rollout,
	} = z
		.object({
			placement_id: z.literal(placementId),
			project_id: z.literal(projectId),
			deployment_id: identifier,
			config_revision: z
				.number()
				.int()
				.positive()
				.max(Number.MAX_SAFE_INTEGER - 1),
			config: placementConfigSchema,
			desired_state: z.enum(["running", "stopped"]).optional(),
			rollout_sources: z
				.array(z.enum(["offline", "online"]))
				.max(2)
				.optional(),
			rollout: rolloutSchema.nullish(),
		})
		.strict()
		.parse(response.result);
	if (
		config.id !== placementId ||
		config.project_id !== projectId ||
		config.deployment_id !== deployment_id ||
		(rollout &&
			(rollout.placement_id !== placementId ||
				rollout.project_id !== projectId))
	)
		throw new Error(
			"The placement configuration does not match its requested project and deployment identity.",
		);
	return {
		placement_id: placementId,
		project_id: projectId,
		deployment_id,
		config_revision,
		config,
		desired_state,
		rollout_sources,
		rollout,
	};
}
export type DeploymentCatalog = {
	events: DeploymentEvent[];
	variables: Record<string, DeploymentVariable[]>;
};
export type InstalledProject = {
	project_id: string;
	project_path: string;
	revision?: string;
	source: "offline" | "online";
	assets?: ProjectArtifactAssets;
	online_metadata_sha256?: string;
	/** Events and variables of the approved metadata installed with this online revision. */
	online_catalog?: DeploymentCatalog;
};

export function canCheckDeploymentStartup(
	installed: InstalledProject,
	existing: PlacementConfiguration | undefined,
	events: DeploymentEvent[],
): boolean {
	return Boolean(
		existing?.desired_state === "running" &&
			existing.config.source === installed.source &&
			(existing.rollout_sources ?? ["offline"]).includes(installed.source) &&
			(installed.source !== "online" || existing.config.resource_grant) &&
			events.length &&
			events.every((event) => event.hosted || event.rollout_supported === true),
	);
}

async function pages(
	call: ManagementCall,
	installed: InstalledProject,
	eventId?: string,
): Promise<unknown[]> {
	const revision = installed.revision;
	if (!revision) throw new Error("Install a pinned snapshot before discovery.");
	let after: string | null = null;
	const items: unknown[] = [];
	do {
		const response = await call({
			type: "artifact",
			request: {
				kind: "describe",
				project_id: installed.project_id,
				revision: installed.revision,
				event_id: eventId ?? null,
				after,
			},
		});
		if (response.state !== "completed")
			throw new Error(
				`The device could not read project revision ${revision}.${rejectionDetail(response)}`,
			);
		const page = z
			.object({
				project_id: z.literal(installed.project_id),
				revision: z.literal(revision),
				event_id: z.literal(eventId ?? null),
				items: z.array(z.unknown()).max(1024),
				next: identifier.nullable(),
			})
			.parse(response.result);
		let previous: string | null = after;
		for (const item of page.items) {
			const { id } = z.object({ id: identifier }).parse(item);
			if (previous !== null && id <= previous)
				throw new Error("Project discovery returned an invalid cursor.");
			previous = id;
			items.push(item);
		}
		if (
			page.next !== null &&
			(page.items.length === 0 || page.next !== previous)
		)
			throw new Error("Project discovery did not make progress.");
		after = page.next;
		if (items.length > (eventId ? 1024 : 512))
			throw new Error("Project inventory exceeds its discovery limit.");
	} while (after !== null);
	return items;
}
export async function discoverOfflineEvents(
	call: ManagementCall,
	installed: InstalledProject,
): Promise<DeploymentEvent[]> {
	if (
		installed.source !== "offline" ||
		!/^[a-f0-9]{64}$/.test(installed.revision ?? "")
	)
		throw new Error("Install an offline snapshot first.");
	return (await pages(call, installed)).map((value) =>
		eventSchema.parse(value),
	);
}
export async function discoverOfflineVariables(
	call: ManagementCall,
	installed: InstalledProject,
	eventId: string,
): Promise<DeploymentVariable[]> {
	return (await pages(call, installed, eventId)).map((value) =>
		variableSchema.parse(value),
	);
}
const byId = <T extends { id: string }>(a: T, b: T) =>
	a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
function samePins(
	left: Pick<DeploymentEvent, "event_version" | "board_version">,
	right: Pick<DeploymentEvent, "event_version" | "board_version">,
): boolean {
	return (
		JSON.stringify(left.event_version) ===
			JSON.stringify(right.event_version) &&
		JSON.stringify(left.board_version) === JSON.stringify(right.board_version)
	);
}
/** In the order the rule checks them. */
export const EVENT_INELIGIBLE_CODES = [
	"paused",
	"latest_flow",
	"canary",
	"variants",
	"type",
	...ROUTE_PROBLEMS,
	...SCHEDULE_PROBLEMS,
	"bot_invalid",
	"hub_schedules",
	"hub_type",
	"flow_error",
	"refuse",
] as const;
export type EventIneligibleCode = (typeof EVENT_INELIGIBLE_CODES)[number];
export type EventReadiness = "listener" | "explicit" | "unsupported";
/**
 * Why an event that follows the Latest flow can't be deployed: the hub is too
 * old, the person can't publish its edits, its Page or start node left the
 * flow, an imported copy has no published flow version, or the pin is unreadable.
 */
export type LatestFlowCase = "hub" | "role" | "target" | "copy" | "other";
/** The `IEvent` fields the rule reads; app catalogues pass the same shape. */
export interface EligibilityEvent {
	/** Read for a bot: its token key must fit 128 characters. */
	id?: string;
	active: boolean;
	event_type: string;
	canary?: unknown;
	variants?: readonly unknown[] | null;
	default_page_id?: string | null;
	event_version?: readonly number[] | null;
	/** Absent: the event follows the Latest flow. */
	board_version?: readonly number[] | null;
	/**
	 * The record's config bytes: the route of an `http` or `api` event, the
	 * schedule of a `cron` event, a bot's settings. Credentials in it are never
	 * read, only whether a bot token is saved.
	 */
	config?: readonly number[] | null;
	/** The schedule half of the config, when the caller already holds it; wins over `config`. */
	schedule?: IScheduleConfig | null;
}
/** Facts from outside the event record: the hub, the approved bundle and a device's own refusal. */
export interface EventEligibilityMetadata {
	/** Known only for an event that follows Latest. */
	latestFlow?: Exclude<LatestFlowCase, "other"> | null;
	/** `false`: the app's hub can't hand schedules to devices. */
	hubSchedules?: boolean;
	/**
	 * Online apps, once the hub's placement list is read: its `event_types`, or
	 * `[]` when it has none. A list without the event's type gives `hub_type` to
	 * an Endpoint, form, quick action or bot. Absent while unknown.
	 */
	hubTypes?: readonly string[];
	ineligibleReason?: string | null;
	deviceRefusal?: string | null;
}
export interface EventEligibility {
	eligible: boolean;
	/** First failing rule, in the order the device checks them; null when eligible. */
	code: EventIneligibleCode | null;
	/**
	 * `flow_error`, `refuse`: the bundle's or the device's sentence.
	 * `route_invalid`: the method or path as written. `route_reserved`: the
	 * path. `bot_invalid`: the setting's key.
	 */
	detail?: string;
	/** How the event runs on a device; null for a type no device runs. */
	kind: EventKind | null;
	hosted: boolean;
	readiness: EventReadiness;
	rolloutSupported: boolean;
	eventVersion: [number, number, number] | null;
	/** Null for an event that follows Latest: the deploy pins the shipped copy. */
	boardVersion: [number, number, number] | null;
	followsLatest: boolean;
	/** Set with `latest_flow`. */
	latestFlow?: LatestFlowCase;
	/** A repeating schedule a device can run. */
	schedule?: DeviceSchedule;
	/** A one-time schedule a device can run, whether or not its time has passed. */
	once?: OnceSchedule;
	/** Set with `schedule_invalid`: what this client's check found. */
	scheduleDetail?: ScheduleDetail;
	/** An `http` or `api` event without a Page: the route a device serves. */
	route?: EventRoute;
	/** A bot whose settings a device reads. */
	bot?: BotFacts;
}
const SERVED_TYPES = ["simple_chat", ...ROUTE_EVENT_TYPES];
const OWN_SERVER_TYPES = ["rest", "mcp"];
const ON_DEMAND_TYPES = ["quick_action", "generic_form"];
const BOT_FEATURES: Record<string, AgentFeature> = {
	telegram: "telegram_bots",
	discord: "discord_bots",
};
/**
 * The event types an online app's hub must name in `event_types` before it
 * hands an event of that type without a Page to a device.
 */
export const HUB_EXPORT_TYPES = [
	"api",
	"quick_action",
	"generic_form",
	"telegram",
	"discord",
] as const;
/** How a device runs the event: a default Page wins whatever the type; null for a type no device runs. */
export function eventKind(
	event: Pick<EligibilityEvent, "event_type" | "default_page_id">,
): EventKind | null {
	if (event.default_page_id || SERVED_TYPES.includes(event.event_type))
		return "served";
	if (OWN_SERVER_TYPES.includes(event.event_type)) return "own_server";
	if (event.event_type === "daemon") return "background";
	if (event.event_type === CRON_EVENT_TYPE) return "scheduled";
	if (ON_DEMAND_TYPES.includes(event.event_type)) return "on_demand";
	return Object.hasOwn(BOT_FEATURES, event.event_type) ? "bot" : null;
}
/** Whether an event of this kind keeps its service at one instance. */
export function limitsInstances(kind: EventKind): boolean {
	return kind !== "served" && kind !== "on_demand";
}
const READINESS: Record<EventKind, EventReadiness> = {
	served: "listener",
	own_server: "listener",
	background: "explicit",
	// Clients before these kinds parse the wire value with a closed enum; `kind` says which.
	scheduled: "explicit",
	on_demand: "explicit",
	bot: "explicit",
};
const parsedConfigs = new WeakMap<object, Record<string, unknown> | null>();
/** The record's config as an object; null when it is none. */
function eventConfig(event: EligibilityEvent): Record<string, unknown> | null {
	const source = event.config;
	if (!source?.length) return null;
	const known = parsedConfigs.get(source);
	if (known !== undefined) return known;
	const config: unknown = parseUint8ArrayToJson(source as number[]);
	const record =
		config && typeof config === "object" && !Array.isArray(config)
			? (config as Record<string, unknown>)
			: null;
	parsedConfigs.set(source, record);
	return record;
}
const scheduleResults = new WeakMap<object, DeviceScheduleResult>();
function scheduleResult(event: EligibilityEvent): DeviceScheduleResult {
	const source = event.schedule === undefined ? event.config : event.schedule;
	if (!source) return deviceSchedule(null);
	const known = scheduleResults.get(source);
	if (known) return known;
	const result = deviceSchedule(
		event.schedule === undefined ? eventConfig(event) : event.schedule,
	);
	scheduleResults.set(source, result);
	return result;
}
/** The route of an `http` or `api` event without a Page; undefined for every other event. */
function routeResult(
	event: EligibilityEvent,
	kind: EventKind | null,
): DeviceRouteResult | undefined {
	return kind === "served" &&
		!event.default_page_id &&
		(ROUTE_EVENT_TYPES as readonly string[]).includes(event.event_type)
		? deviceRoute(eventConfig(event))
		: undefined;
}
function botResult(
	event: EligibilityEvent,
	kind: EventKind | null,
): DeviceBotResult | undefined {
	return kind === "bot"
		? (deviceBot(event.event_type, eventConfig(event), event.id) ?? undefined)
		: undefined;
}
function hubLacksType(
	event: EligibilityEvent,
	metadata: EventEligibilityMetadata,
): boolean {
	return (
		metadata.hubTypes !== undefined &&
		!event.default_page_id &&
		(HUB_EXPORT_TYPES as readonly string[]).includes(event.event_type) &&
		!metadata.hubTypes.includes(event.event_type)
	);
}
/** The device's event rule (`sa/deployment.rs`, `sa/runtime.rs`), shared by online discovery and every app view. */
export function eventEligibility(
	event: EligibilityEvent,
	metadata: EventEligibilityMetadata = {},
): EventEligibility {
	const kind = eventKind(event);
	const hosted = kind === "served";
	const eventVersion = version.safeParse(event.event_version);
	const followsLatest = event.board_version == null;
	const boardVersion = version.safeParse(event.board_version);
	const latestFlow: LatestFlowCase | null =
		!eventVersion.success || (!followsLatest && !boardVersion.success)
			? "other"
			: followsLatest
				? (metadata.latestFlow ?? null)
				: null;
	const route = routeResult(event, kind);
	const schedule = kind === "scheduled" ? scheduleResult(event) : undefined;
	const bot = botResult(event, kind);
	const code: EventIneligibleCode | null = !event.active
		? "paused"
		: latestFlow
			? "latest_flow"
			: event.canary
				? "canary"
				: event.variants?.length
					? "variants"
					: kind === null
						? "type"
						: route && !route.ok
							? route.problem
							: schedule && !schedule.ok
								? schedule.problem
								: bot && !bot.ok
									? bot.problem
									: schedule && metadata.hubSchedules === false
										? "hub_schedules"
										: hubLacksType(event, metadata)
											? "hub_type"
											: metadata.ineligibleReason
												? "flow_error"
												: metadata.deviceRefusal
													? "refuse"
													: null;
	const detail =
		code === "flow_error"
			? (metadata.ineligibleReason ?? undefined)
			: code === "refuse"
				? (metadata.deviceRefusal ?? undefined)
				: route && !route.ok && code === route.problem
					? route.detail
					: bot && !bot.ok && code === bot.problem
						? bot.detail
						: undefined;
	return {
		eligible: code === null,
		code,
		...(detail ? { detail } : {}),
		kind,
		hosted,
		readiness: kind ? READINESS[kind] : "unsupported",
		rolloutSupported:
			kind !== null &&
			schedule?.ok !== false &&
			route?.ok !== false &&
			bot?.ok !== false,
		eventVersion: eventVersion.success ? eventVersion.data : null,
		boardVersion: boardVersion.success ? boardVersion.data : null,
		followsLatest,
		...(code === "latest_flow" && latestFlow ? { latestFlow } : {}),
		...(schedule?.ok && schedule.schedule
			? { schedule: schedule.schedule }
			: {}),
		...(schedule?.ok && schedule.once ? { once: schedule.once } : {}),
		...(code === "schedule_invalid" &&
		schedule &&
		!schedule.ok &&
		schedule.detail
			? { scheduleDetail: schedule.detail }
			: {}),
		...(route?.ok ? { route: route.route } : {}),
		...(bot?.ok ? { bot: bot.bot } : {}),
	};
}
/**
 * The agent flags a device needs to run this event: none for a Page. An
 * `http` event outside the strict route form needs `api_events` too, since
 * older agents read only `{path: "/…", method}` and fail at start.
 */
export function requiredFeatures(event: EligibilityEvent): AgentFeature[] {
	const kind = eventKind(event);
	if (!kind || event.default_page_id) return [];
	switch (kind) {
		case "served": {
			if (event.event_type === "api") return ["api_events"];
			const route = routeResult(event, kind);
			return route && !(route.ok && route.strict) ? ["api_events"] : [];
		}
		case "scheduled": {
			const schedule = scheduleResult(event);
			return schedule.ok && schedule.once
				? ["scheduled_events", "scheduled_once"]
				: ["scheduled_events"];
		}
		case "on_demand":
			return ["on_demand_events"];
		case "bot":
			return [BOT_FEATURES[event.event_type]];
		default:
			return [];
	}
}
/** The first flag the agent lacks for this event; `unknown` when its flags are not known. */
export function missingFeature(
	event: EligibilityEvent,
	features: AgentFeatures | undefined,
): AgentFeature | "unknown" | null {
	const needed = requiredFeatures(event);
	if (!needed.length) return null;
	if (!features) return "unknown";
	return needed.find((feature) => features[feature] !== 1) ?? null;
}
function onlineEvent(event: IEvent): DeploymentEvent {
	const rule = eventEligibility(event);
	const deviceCode = DEVICE_INELIGIBLE_CODES.find((code) => code === rule.code);
	return eventSchema.parse({
		id: event.id,
		name: event.name.slice(0, 120),
		event_type: event.event_type,
		event_version: rule.eventVersion,
		board_version: rule.boardVersion,
		hosted: rule.hosted,
		readiness_kind: rule.readiness,
		rollout_supported: rule.rolloutSupported,
		eligible: rule.eligible,
		...(rule.kind ? { kind: rule.kind } : {}),
		...(rule.schedule
			? {
					schedule: {
						expression: rule.schedule.expression,
						timezone: rule.schedule.timezone,
					},
				}
			: {}),
		...(rule.route ? { route: rule.route } : {}),
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
		...(deviceCode ? { ineligible_code: deviceCode } : {}),
	});
}
/**
 * The variables a board and its layers let a device set (exposed, or set at run
 * time). Throws on one a placement cannot configure and on two layers that
 * define the same variable differently.
 */
export function boardVariables(
	board: Pick<IBoard, "variables" | "layers">,
): DeploymentVariable[] {
	return mergeVariables(
		[
			Object.values(board.variables ?? {})
				.concat(
					...Object.values(board.layers ?? {}).map((layer) =>
						Object.values(layer.variables ?? {}),
					),
				)
				.filter((v) => v.exposed || v.runtime_configured)
				.map((v) => {
					const parsed = variableSchema.safeParse({
						id: v.id,
						name: String(v.name ?? "").slice(0, 120),
						data_type: v.data_type,
						value_type: v.value_type,
						secret: v.secret,
					});
					if (!parsed.success)
						throw new Error(
							`Variable ${JSON.stringify(String(v.name || v.id).slice(0, 64))} has an identifier or type that a device placement cannot configure.`,
						);
					return parsed.data;
				}),
		],
		(variable) =>
			`The board and its layers define variable ${variable.name} differently.`,
	);
}
/** Deployment choices from the exact approved metadata bundle the device verifies. */
export function approvedOnlineCatalog(
	documents: Record<string, unknown>,
): DeploymentCatalog {
	const events: DeploymentEvent[] = [];
	const variables: Record<string, DeploymentVariable[]> = Object.create(null);
	for (const [key, value] of Object.entries(documents)) {
		if (!key.startsWith("events/")) continue;
		if (events.length >= 512)
			throw new Error("Approved project metadata exceeds 512 events.");
		const record = value as IEvent;
		const event = onlineEvent(record);
		if (
			!event.event_version ||
			!event.board_version ||
			key !== `events/${event.id}/versions/${event.event_version.join("/")}` ||
			Object.hasOwn(variables, event.id)
		)
			throw new Error(
				`Approved metadata document ${key} does not match its event identity. Prepare the project again.`,
			);
		const boardKey = `boards/${record.board_id}/versions/${event.board_version.join("/")}`;
		const board = documents[boardKey] as IBoard | undefined;
		if (
			!board ||
			board.id !== record.board_id ||
			JSON.stringify(board.version) !== JSON.stringify(event.board_version)
		)
			throw new Error(
				`Approved metadata for event ${event.id} is missing board ${boardKey}. Prepare the project again.`,
			);
		const boardLabel = `${record.board_id} ${event.board_version.join(".")}`;
		// The device accepts the bundle, so one unconfigurable board only blocks its own event.
		try {
			variables[event.id] = boardVariables(board);
			events.push(event);
		} catch (error) {
			variables[event.id] = [];
			events.push({
				...event,
				eligible: false,
				ineligible_reason:
					`Board ${boardLabel} cannot be deployed: ${error instanceof Error ? error.message : String(error)}`.slice(
						0,
						480,
					),
			});
		}
	}
	return { events: events.sort(byId), variables };
}
function approvedCatalog(installed: InstalledProject): DeploymentCatalog {
	if (installed.source !== "online" || !installed.online_catalog)
		throw new Error(
			"Prepare and install this online project again so deployment uses its approved executable metadata.",
		);
	return installed.online_catalog;
}
/** Online events come from the installed approved metadata, never the live cloud. */
export function discoverOnlineEvents(
	installed: InstalledProject,
): DeploymentEvent[] {
	return approvedCatalog(installed).events;
}
function requireApprovedEvent(
	catalog: DeploymentCatalog,
	selected: DeploymentEvent,
): void {
	const approved = catalog.events.find((event) => event.id === selected.id);
	if (!approved?.eligible || !samePins(approved, selected))
		throw new Error(
			`Event ${selected.id} ${selected.event_version?.join(".") ?? "(unpinned)"} is not in the approved metadata installed on the device. Prepare and install the project again.`,
		);
}
export function discoverOnlineVariables(
	installed: InstalledProject,
	selected: DeploymentEvent,
): DeploymentVariable[] {
	const catalog = approvedCatalog(installed);
	requireApprovedEvent(catalog, selected);
	return catalog.variables[selected.id] ?? [];
}
function previousSecrets(
	groups: DeploymentVariable[][],
	existing: PlacementConfiguration,
): DeploymentVariable[] {
	const secrets = new Map<string, DeploymentVariable>();
	for (const variable of groups.flat())
		if (
			variable.secret &&
			Object.hasOwn(existing.config.secret_overrides, variable.id) &&
			!secrets.has(variable.id)
		)
			secrets.set(variable.id, variable);
	return [...secrets.values()];
}
/** Secret definitions the existing revision's stored references were written for. */
export async function discoverPreviousOfflineVariables(
	call: ManagementCall,
	installed: InstalledProject,
	existing: PlacementConfiguration,
): Promise<DeploymentVariable[]> {
	if (
		!Object.keys(existing.config.secret_overrides).length ||
		existing.config.revision === installed.revision
	)
		return [];
	const previous = { ...installed, revision: existing.config.revision };
	const groups: DeploymentVariable[][] = [];
	for (const binding of existing.config.events)
		groups.push(
			await discoverOfflineVariables(call, previous, binding.event_id),
		);
	return previousSecrets(groups, existing);
}
export async function discoverPreviousOnlineVariables(
	events: IEventState,
	boards: IBoardState,
	installed: InstalledProject,
	existing: PlacementConfiguration,
): Promise<DeploymentVariable[]> {
	if (!Object.keys(existing.config.secret_overrides).length) return [];
	const catalog = approvedCatalog(installed);
	const groups: DeploymentVariable[][] = [];
	for (const binding of existing.config.events) {
		const approved = catalog.events.find(
			(event) => event.id === binding.event_id,
		);
		if (approved && samePins(approved, binding)) {
			groups.push(catalog.variables[binding.event_id] ?? []);
			continue;
		}
		const event = await events.getEventAuthoritative(
			installed.project_id,
			binding.event_id,
			binding.event_version,
		);
		if (
			event.id !== binding.event_id ||
			JSON.stringify(event.event_version) !==
				JSON.stringify(binding.event_version)
		)
			throw new Error(
				`Published event ${binding.event_id} differs from its deployed version.`,
			);
		const board = await boards.getBoardAuthoritative(
			installed.project_id,
			event.board_id,
			binding.board_version,
		);
		if (
			board.id !== event.board_id ||
			JSON.stringify(board.version) !== JSON.stringify(binding.board_version)
		)
			throw new Error("Published board pins differ.");
		groups.push(boardVariables(board));
	}
	return previousSecrets(groups, existing);
}
/** The earlier definition when a kept secret reference was written for another type. */
export function changedSecretType(
	previous: readonly DeploymentVariable[] | undefined,
	current: DeploymentVariable,
): DeploymentVariable | undefined {
	const old = previous?.find((variable) => variable.id === current.id);
	return old &&
		(old.data_type !== current.data_type ||
			old.value_type !== current.value_type)
		? old
		: undefined;
}
export function mergeVariables(
	groups: DeploymentVariable[][],
	conflict = (variable: DeploymentVariable) =>
		`These events disagree about variable ${variable.name}. Deploy them separately.`,
): DeploymentVariable[] {
	const rows = new Map<string, DeploymentVariable>();
	for (const variable of groups.flat()) {
		const previous = rows.get(variable.id);
		if (previous && JSON.stringify(previous) !== JSON.stringify(variable))
			throw new Error(conflict(variable));
		rows.set(variable.id, variable);
	}
	return [...rows.values()].sort((a, b) =>
		a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
	);
}
function isLiteralText(variable: DeploymentVariable): boolean {
	return (
		variable.value_type === "Normal" &&
		["String", "PathBuf", "Date"].includes(variable.data_type)
	);
}
export function variableText(
	variable: DeploymentVariable,
	value: unknown,
): string {
	return isLiteralText(variable) && typeof value === "string"
		? value
		: JSON.stringify(value);
}
export function variableValue(
	variable: DeploymentVariable,
	text: string,
): unknown {
	if (isLiteralText(variable)) return text;
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		throw new Error(`Enter valid JSON for ${variable.name}.`);
	}
	validateVariableValue(variable, value);
	return value;
}
/** Checks an existing public value without converting it to another type. */
export function validateVariableValue(
	variable: DeploymentVariable,
	value: unknown,
): void {
	const scalar = (item: unknown) => {
		if (
			["String", "PathBuf", "Date"].includes(variable.data_type) &&
			typeof item !== "string"
		)
			throw new Error(`${variable.name} requires text.`);
		if (variable.data_type === "Boolean" && typeof item !== "boolean")
			throw new Error(`${variable.name} requires true or false.`);
		if (
			["Integer", "Byte"].includes(variable.data_type) &&
			(typeof item !== "number" ||
				!Number.isSafeInteger(item) ||
				(variable.data_type === "Byte" && (item < 0 || item > 255)))
		)
			throw new Error(`${variable.name} requires an integer in range.`);
		if (
			variable.data_type === "Float" &&
			(typeof item !== "number" || !Number.isFinite(item))
		)
			throw new Error(`${variable.name} requires a finite number.`);
	};
	if (variable.value_type === "Normal") scalar(value);
	else if (["Array", "HashSet"].includes(variable.value_type)) {
		if (!Array.isArray(value))
			throw new Error(`${variable.name} requires a JSON array.`);
		for (const item of value) scalar(item);
	} else if (variable.value_type === "HashMap") {
		if (!value || typeof value !== "object" || Array.isArray(value))
			throw new Error(`${variable.name} requires a JSON object.`);
		for (const item of Object.values(value)) scalar(item);
	} else throw new Error(`${variable.name} has an unsupported value type.`);
}
export type DeploymentPlan = {
	config: Record<string, unknown>;
	/** Configuration revision the plan replaces; 0 creates a placement. */
	expected_revision: number;
	rollout_id?: string;
	steps: {
		id: string;
		command: Record<string, unknown>;
		attempted?: boolean;
	}[];
};
type PlanInput = {
	installed: InstalledProject;
	existing?: PlacementConfiguration;
	healthChecked?: boolean;
	removeOverrides?: string[];
	placement: string;
	deployment: string;
	events: DeploymentEvent[];
	variables: DeploymentVariable[];
	/** Secret definitions of the existing revision; kept references must still match them. */
	previousVariables?: DeploymentVariable[];
	overrides: Record<string, string>;
	host: string;
	port: number;
	replicas: number;
	serviceToken: string;
	/** Explicitly expose selected forms and quick actions through the service listener. */
	hostOnDemand?: boolean;
	/** Undefined preserves existing authentication and defaults to a token for new services. */
	serviceAuthentication?: "token" | "none";
	/** Undefined keeps the selected certificate; null removes the device override. */
	tlsCertificateId?: string | null;
	resourceGrant?: Record<string, unknown>;
	/** Undefined preserves existing settings; null explicitly disables buffering. */
	offlineWrites?: OfflineWritesConfig | null;
	/** Undefined preserves existing limits; null selects the trusted process profile. */
	resourceLimits?: PlacementResources | null;
};
/**
 * The settings a placement may carry: the boards' variables, and one secret
 * per selected bot for its token. A flow variable whose id is a bot token key
 * never takes a value; a device refuses such a flow.
 */
function placementDefinitions(
	input: PlanInput,
): Map<string, DeploymentVariable> {
	const definitions = new Map(
		input.variables
			.filter((variable) => botTokenEventId(variable.id) === null)
			.map((variable) => [variable.id, variable]),
	);
	for (const event of input.events)
		if (event.kind === "bot") {
			const token = botTokenVariable(event);
			definitions.set(token.id, token);
		}
	return definitions;
}
/** The trimmed bot token of an override, after the shape check a device makes. */
function botTokenText(input: PlanInput, id: string, text: string): string {
	const eventId = botTokenEventId(id);
	const event = input.events.find((item) => item.id === eventId);
	const provider = event && botProvider(event.event_type);
	if (!event || !provider) return text;
	const problem = botTokenProblem(provider, text);
	if (problem)
		throw new Error(
			`The bot token of ${event.name} doesn't look like a ${provider === "telegram" ? "Telegram" : "Discord"} bot token. Enter it again under Settings.`,
		);
	return text.trim();
}
function placementVariables(input: PlanInput) {
	const removed = new Set(input.removeOverrides ?? []);
	for (const id of removed) identifier.parse(id);
	const plain: Record<string, unknown> = Object.create(null);
	const references: Record<string, string> = Object.create(null);
	for (const [id, value] of Object.entries(
		input.existing?.config.variables ?? {},
	))
		if (!removed.has(id)) plain[id] = value;
	for (const [id, name] of Object.entries(
		input.existing?.config.secret_overrides ?? {},
	))
		if (!removed.has(id)) references[id] = name;
	const secrets: { name: string; value: string }[] = [];
	const replaced = new Set<string>();
	const definitions = placementDefinitions(input);
	for (const [id, text] of Object.entries(input.overrides)) {
		const definition = definitions.get(id);
		if (!definition)
			throw new Error(
				`Variable ${id} is outside the selected events. Select its event again or clear its value.`,
			);
		// Empty secret inputs mean keep the opaque reference, never read or replace it.
		if (definition.secret && text === "") continue;
		const value = variableValue(definition, botTokenText(input, id, text));
		if (definition.secret) {
			const name = `variable-${crypto.randomUUID()}`;
			delete plain[id];
			references[id] = name;
			replaced.add(id);
			secrets.push({ name, value: JSON.stringify(value) });
		} else {
			delete references[id];
			plain[id] = value;
		}
	}
	for (const id of [...Object.keys(plain), ...Object.keys(references)]) {
		const definition = definitions.get(id);
		if (!definition)
			throw new Error(
				`Stored override ${id} is outside the selected events. Select its event again or remove the override.`,
			);
		if (definition.secret !== Object.hasOwn(references, id))
			throw new Error(
				`Stored override ${id} changed between public and secret. Replace or remove it.`,
			);
		const previous = replaced.has(id)
			? undefined
			: changedSecretType(input.previousVariables, definition);
		if (Object.hasOwn(references, id) && previous)
			throw new Error(
				`Stored secret ${id} was written for ${previous.data_type}/${previous.value_type}, but the selected event expects ${definition.data_type}/${definition.value_type}. Enter a new value or remove the override.`,
			);
		if (Object.hasOwn(plain, id)) {
			try {
				validateVariableValue(definition, plain[id]);
			} catch {
				throw new Error(
					`Stored override ${id} is incompatible with its selected type. Replace or remove it.`,
				);
			}
		}
	}
	for (const event of input.events)
		if (
			event.kind === "bot" &&
			!Object.hasOwn(references, botTokenKey(event.id))
		)
			throw new Error(
				`Bot ${event.name} needs its bot token. Enter it under Settings.`,
			);
	return { plain, references, secrets };
}
/** A discovery row without `kind` comes from an agent before kinds: anything not hosted keeps one instance. */
function keepsOneInstance(event: DeploymentEvent): boolean {
	return event.kind ? limitsInstances(event.kind) : !event.hosted;
}
function retainedSettings(
	existing?: PlacementConfiguration,
): Record<string, unknown> {
	if (!existing) return {};
	const {
		hosting: _hosting,
		resource_grant: _grant,
		offline_writes: _offlineWrites,
		resources: _resources,
		...retained
	} = existing.config;
	return retained;
}
/** True when an update turns buffering off or stops buffering a previously buffered resource. */
export function removesOfflineBuffering(
	previous: OfflineWritesConfig | null | undefined,
	next: OfflineWritesConfig | null | undefined,
): boolean {
	if (!previous) return false;
	if (!next) return true;
	return (
		previous.tables.some(
			(table) =>
				!next.tables.some(
					(candidate) =>
						candidate.purpose === table.purpose &&
						candidate.database === table.database &&
						candidate.table === table.table,
				),
		) ||
		previous.files.some(
			(file) =>
				!next.files.some(
					(candidate) =>
						candidate.purpose === file.purpose &&
						candidate.prefix === file.prefix,
				),
		)
	);
}
/** Queued writes are replayed only while their resources stay buffered. */
export async function assertOfflineQueuesDrained(
	call: ManagementCall,
	placementId: string,
): Promise<void> {
	let pending: number;
	try {
		pending = (await readOfflineQueues(call, placementId)).reduce(
			(total, queue) => total + queue.pending_count,
			0,
		);
	} catch (error) {
		throw new Error(
			`Removing buffered writes from placement ${placementId} requires reading its offline queue first: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
	if (pending > 0)
		throw new Error(
			`Placement ${placementId} still has ${pending} buffered writes waiting to replay. Keep its buffered tables and directories until the queue is empty, or retry or skip the queued writes under Offline write queues first.`,
		);
}
/** One Apply or StageRollout carries the whole configuration in a single management message. */
export const DEPLOYMENT_CONFIG_BYTES = 12_000;
function jsonBytes(value: unknown): number {
	return new TextEncoder().encode(JSON.stringify(value)).length;
}
/** Bytes that Bit and node-package pins add to every placement configuration of a project. */
export function deploymentPinBytes(
	assets: Pick<ProjectArtifactAssets, "bit_pins" | "package_pins">,
): number {
	return jsonBytes({
		bit_pins: assets.bit_pins,
		package_pins: assets.package_pins,
	});
}
function deploymentSizeError(
	config: Record<string, unknown>,
	bytes: number,
): Error {
	const pins = deploymentPinBytes({
		bit_pins: (config.bit_pins ?? []) as ProjectArtifactAssets["bit_pins"],
		package_pins: (config.package_pins ??
			[]) as ProjectArtifactAssets["package_pins"],
	});
	const values = jsonBytes(config.variables ?? {});
	return new Error(
		`This deployment needs ${bytes} bytes, but one remote management message carries at most ${DEPLOYMENT_CONFIG_BYTES}. Bit and node-package pins of the whole project use ${pins} bytes and public variable overrides use ${values} bytes. ${
			pins >= values
				? "Remove Bits or node packages the project no longer uses and prepare it again."
				: "Shorten the public variable overrides."
		}`,
	);
}
export function createDeploymentPlan(input: PlanInput): DeploymentPlan {
	const resourceLimits =
		input.resourceLimits === undefined
			? input.existing?.config.resources
			: input.resourceLimits;
	const checkedResources =
		resourceLimits == null
			? undefined
			: placementResourcesSchema.parse(resourceLimits);
	const offlineWrites =
		input.offlineWrites === undefined
			? input.existing?.config.offline_writes
			: input.offlineWrites;
	if (offlineWrites != null && input.installed.source !== "online")
		throw new Error(
			"Offline write buffering applies only to online project placements.",
		);
	const checkedOfflineWrites =
		offlineWrites == null
			? undefined
			: offlineWritesSchema.parse(offlineWrites);
	if (checkedOfflineWrites && input.replicas !== 1)
		throw new Error(
			`Offline write buffering requires a placement with one replica, but this placement allows ${input.replicas}. Turn off buffering or use a single-replica placement.`,
		);
	const { installed, events, existing } = input;
	identifier.parse(input.placement);
	identifier.parse(input.deployment);
	identifier.parse(installed.project_id);
	if (existing) {
		assertBoundedJson(existing.config, 16 * 1024);
		placementConfigSchema.parse(existing.config);
		z.number()
			.int()
			.positive()
			.max(Number.MAX_SAFE_INTEGER - 1)
			.parse(existing.config_revision);
	}
	if (input.placement === "device")
		throw new Error("Choose a placement ID other than device.");
	if (
		existing &&
		(existing.placement_id !== input.placement ||
			existing.config.id !== input.placement ||
			existing.project_id !== installed.project_id ||
			existing.deployment_id !== input.deployment ||
			existing.config.deployment_id !== input.deployment ||
			existing.config.project_id !== installed.project_id ||
			existing.config.source !== installed.source)
	)
		throw new Error(
			`Placement ${existing.placement_id} keeps its identity, project and source during an update. Reload the placement.`,
		);
	if (
		existing &&
		(input.replicas !== existing.config.max_replicas ||
			(input.resourceGrant !== undefined &&
				!equivalent(input.resourceGrant, existing.config.resource_grant)))
	)
		throw new Error(
			"This update keeps the placement replica limit and resource and billing approvals. Use the placement controls to change them separately.",
		);
	if (
		!events.length ||
		events.length > 64 ||
		events.some(
			(event) =>
				!eventSchema.safeParse(event).success ||
				!event.eligible ||
				!event.event_version ||
				!event.board_version,
		) ||
		new Set(events.map((event) => event.id)).size !== events.length
	)
		throw new Error("Select 1 to 64 published, supported events.");
	if (installed.source === "online" && installed.online_catalog)
		for (const event of events)
			requireApprovedEvent(installed.online_catalog, event);
	const resourceGrant = existing
		? (existing.config.resource_grant ?? undefined)
		: input.resourceGrant;
	if (installed.source === "online" && !resourceGrant)
		throw new Error("Select a resource approval for the online project.");
	if (
		installed.source === "online" &&
		!/^[a-f0-9]{64}$/.test(
			installed.online_metadata_sha256 ??
				(typeof existing?.config.online_metadata_sha256 === "string"
					? existing.config.online_metadata_sha256
					: ""),
		)
	)
		throw new Error(
			"Prepare and install controller-approved executable metadata before deploying this online project.",
		);
	const hosted =
		events.some((event) => event.hosted) ||
		Boolean(
			(input.hostOnDemand === true || existing?.config.hosting) &&
				events.some((event) => event.kind === "on_demand"),
		);
	if (
		!Number.isInteger(input.replicas) ||
		input.replicas < 1 ||
		input.replicas > 32 ||
		(input.replicas > 1 && (!hosted || events.some(keepsOneInstance)))
	)
		throw new Error(
			"Only HTTP, chat and Page services can use multiple replicas. Schedules, bots, background and own-server events keep a service at one.",
		);
	const previousHosting = existing?.config.hosting ?? undefined;
	const authentication =
		input.serviceAuthentication ?? previousHosting?.authentication ?? "token";
	const { auth_secret: _previousAuthSecret, ...hostingSettings } =
		previousHosting ?? {};
	const replaceToken =
		hosted &&
		authentication === "token" &&
		(!previousHosting?.auth_secret || input.serviceToken !== "");
	if (
		hosted &&
		(!z.string().ip().safeParse(input.host).success ||
			!Number.isInteger(input.port) ||
			input.port < 1 ||
			input.port > 65535)
	)
		throw new Error("Set a listener IP and a port from 1 to 65535.");
	if (replaceToken && !/^[\x21-\x7e]{32,4096}$/.test(input.serviceToken))
		throw new Error(
			"Set a service token of at least 32 printable characters without whitespace.",
		);
	assertOneVersionPerPackage(installed.assets?.package_pins ?? []);
	const { plain, references, secrets } = placementVariables(input);
	// A rotated token gets a fresh name so the running revision keeps its current token.
	const authSecret =
		authentication === "none"
			? undefined
			: !previousHosting
				? "service-access"
				: replaceToken
					? `service-access-${crypto.randomUUID()}`
					: previousHosting.auth_secret;
	if (replaceToken && authSecret)
		secrets.push({ name: authSecret, value: input.serviceToken });
	for (const secret of secrets) {
		const bytes = new TextEncoder().encode(secret.value).length;
		if (bytes < 1 || bytes > 4096)
			throw new Error(
				"Each serialized secret must contain 1 to 4096 UTF-8 bytes. Shorten the value before deploying.",
			);
	}
	const config: Record<string, unknown> = {
		...retainedSettings(existing),
		id: input.placement,
		project_id: installed.project_id,
		deployment_id: input.deployment,
		revision:
			installed.revision ?? existing?.config.revision ?? crypto.randomUUID(),
		source: installed.source,
		project_path: installed.project_path,
		...(installed.source === "online"
			? {
					online_metadata_sha256:
						installed.online_metadata_sha256 ??
						existing?.config.online_metadata_sha256,
				}
			: {}),
		events: events.map((event) => ({
			event_id: event.id,
			event_version: event.event_version,
			board_version: event.board_version,
		})),
		variables: plain,
		secret_overrides: references,
		max_replicas: input.replicas,
		bit_pins: installed.assets?.bit_pins ?? existing?.config.bit_pins ?? [],
		package_pins:
			installed.assets?.package_pins ?? existing?.config.package_pins ?? [],
		...(hosted
			? {
					hosting: {
						...hostingSettings,
						host: input.host,
						port: input.port,
						max_in_flight: previousHosting?.max_in_flight ?? 64,
						request_timeout_secs: previousHosting?.request_timeout_secs ?? 300,
						...(authentication === "none" || previousHosting?.authentication
							? { authentication }
							: {}),
						...(authSecret ? { auth_secret: authSecret } : {}),
					},
				}
			: {}),
		...(resourceGrant ? { resource_grant: resourceGrant } : {}),
	};
	if (input.tlsCertificateId !== undefined) {
		if (input.tlsCertificateId === null)
			Reflect.deleteProperty(config, "tls_certificate_id");
		else
			config.tls_certificate_id = z
				.string()
				.uuid()
				.parse(input.tlsCertificateId);
	}
	if (checkedOfflineWrites) config.offline_writes = checkedOfflineWrites;
	if (checkedResources) config.resources = checkedResources;
	if (existing) {
		const { hosting, resource_grant, offline_writes, resources, ...settings } =
			existing.config;
		const normalized = {
			...settings,
			...(hosting == null ? {} : { hosting }),
			...(resource_grant == null ? {} : { resource_grant }),
			...(offline_writes == null ? {} : { offline_writes }),
			...(resources == null ? {} : { resources }),
		};
		if (equivalent(config, normalized))
			throw new Error(
				"This update has no configuration changes. Choose a new snapshot, event version or variable override.",
			);
	}
	placementConfigSchema.parse(config);
	assertBoundedJson(config, Number.POSITIVE_INFINITY);
	if (jsonBytes(config) > DEPLOYMENT_CONFIG_BYTES)
		throw deploymentSizeError(config, jsonBytes(config));
	const expected = existing?.config_revision ?? 0;
	if (
		input.healthChecked &&
		!canCheckDeploymentStartup(installed, existing, events)
	)
		throw new Error(
			"Automatic startup checks require a running HTTP, chat or Page placement and device support for its project source.",
		);
	const rolloutId = input.healthChecked ? crypto.randomUUID() : undefined;
	const apply = {
		id: rolloutId ?? crypto.randomUUID(),
		command: rolloutId
			? {
					type: "stage_rollout",
					config,
					expected_revision: expected,
					stabilization_seconds: 10,
					deadline_seconds: 120,
				}
			: ({
					type: "apply",
					config,
					expected_revision: expected,
					start: false,
				} as Record<string, unknown>),
	};
	const secretSteps = secrets.map((secret) => ({
		id: crypto.randomUUID(),
		command: rolloutId
			? {
					type: "rollout_secret",
					rollout_id: rolloutId,
					...secret,
				}
			: {
					type: "set_secret",
					placement_id: input.placement,
					expected_revision: expected + 1,
					...secret,
				},
	}));
	// Apply stops the placement. Start is a separate operation after secrets publish.
	const steps = [apply, ...secretSteps];
	if (rolloutId)
		steps.push({
			id: crypto.randomUUID(),
			command: { type: "activate_rollout", rollout_id: rolloutId },
		});
	const largest = Math.max(...steps.map((step) => jsonBytes(step.command)));
	if (largest > DEPLOYMENT_CONFIG_BYTES)
		throw deploymentSizeError(config, largest);
	return { config, steps, expected_revision: expected, rollout_id: rolloutId };
}
/** `rejection` is a retryable coded refusal; older agents send none. */
function applyFailure(
	plan: DeploymentPlan,
	rejection?: ManagementRejection,
): string {
	const subject = plan.expected_revision
		? `The update of placement ${String(plan.config.id)}`
		: "Placement creation";
	if (rejection)
		return `${subject} was not confirmed. Device response: ${rejection.error} Retry to confirm the same operations.`;
	return plan.expected_revision
		? `${subject} was not confirmed. Reload the placement and try again.`
		: `${subject} was not confirmed. Its identity may already exist.`;
}

async function executeRolloutPlan(
	call: ManagementCall,
	plan: DeploymentPlan,
	signal?: AbortSignal,
	onStatus?: (status: DeploymentRolloutStatus) => void,
): Promise<void> {
	for (const step of plan.steps) {
		signal?.throwIfAborted();
		let response = step.attempted
			? await call({ type: "operation", operation_id: step.id })
			: undefined;
		signal?.throwIfAborted();
		if (!response || response.state === "rejected") {
			step.attempted = true;
			response = await call(step.command, step.id);
		}
		signal?.throwIfAborted();
		if (response.state === "rejected") {
			const rejection = finalRejection(response);
			if (step.command.type === "stage_rollout") {
				const current = await readExistingDeployment(
					call,
					String(plan.config.id),
					String(plan.config.project_id),
				).catch((error: unknown) => {
					if (rejection) return undefined;
					throw error;
				});
				if (current && current.config_revision !== plan.expected_revision)
					throw new StaleDeploymentRevisionError(
						String(plan.config.id),
						plan.expected_revision,
						current.config_revision,
					);
				if (current?.desired_state === "stopped")
					throw new DeploymentReviewRequiredError(
						"The placement was stopped after this update was prepared. Reload it and review the update before continuing.",
					);
				if (
					current?.rollout &&
					current.rollout.rollout_id !== plan.rollout_id &&
					["staged", "validating", "activating", "rolling_back"].includes(
						current.rollout.state,
					)
				)
					throw new DeploymentReviewRequiredError(
						"Another update is already in progress for this placement. Reload it to review that rollout.",
					);
				if (rejection)
					throw new DeploymentRejectedError(
						rejection,
						`the update of placement ${String(plan.config.id)}`,
					);
			}
			const current = await readDeploymentRollout(call, {
				rollout_id: String(plan.rollout_id),
				placement_id: String(plan.config.id),
				project_id: String(plan.config.project_id),
			}).catch(() => undefined);
			signal?.throwIfAborted();
			if (current) {
				onStatus?.(current);
				if (["rolled_back", "failed", "cancelled"].includes(current.state))
					throw new DeploymentRolloutEndedError(current);
				if (current.state === "healthy") return;
			}
			if (rejection)
				throw new DeploymentRejectedError(
					rejection,
					step.command.type === "rollout_secret"
						? `staged secret ${String(step.command.name)}`
						: "activation of the staged update",
					"Discard the staged update, then prepare the update again.",
				);
			throw new Error(
				`The device rejected this rollout operation.${rejectionDetail(response)} Retry to confirm its journal and current status.`,
			);
		}
		if (
			response.operation_id !== step.id ||
			!["accepted", "completed"].includes(response.state) ||
			response.result.rollout_id !== plan.rollout_id ||
			response.result.placement_id !== plan.config.id
		)
			throw new Error(
				"The device returned an unconfirmed or differently scoped rollout operation.",
			);
		if (step.command.type === "rollout_secret") {
			if (
				response.result.name !== step.command.name ||
				response.result.secret !== "completed"
			)
				throw new Error(
					"The device did not confirm the staged secret for this rollout.",
				);
		} else if (
			response.result.project_id !== plan.config.project_id ||
			response.result.state !==
				(step.command.type === "stage_rollout" ? "staged" : "validating")
		) {
			throw new Error(
				"The device returned a different rollout state or project.",
			);
		}
	}
}

export async function waitForDeploymentRollout(
	call: ManagementCall,
	scope: Pick<
		DeploymentRolloutStatus,
		"rollout_id" | "placement_id" | "project_id"
	>,
	signal?: AbortSignal,
	onStatus?: (status: DeploymentRolloutStatus) => void,
): Promise<DeploymentRolloutStatus> {
	for (let attempts = 0; attempts < 300; attempts++) {
		signal?.throwIfAborted();
		const status = await readDeploymentRollout(call, scope);
		signal?.throwIfAborted();
		onStatus?.(status);
		if (status.state === "healthy") return status;
		if (["rolled_back", "failed", "cancelled"].includes(status.state))
			throw new DeploymentRolloutEndedError(status);
		await new Promise<void>((resolve, reject) => {
			const abort = () => {
				clearTimeout(timer);
				reject(signal?.reason ?? new Error("Rollout observation cancelled."));
			};
			const timer = setTimeout(() => {
				signal?.removeEventListener("abort", abort);
				resolve();
			}, 1000);
			signal?.addEventListener("abort", abort, { once: true });
			if (signal?.aborted) abort();
		});
	}
	throw new Error(
		"The device is still processing this rollout. Reconnect and retry to read its durable status.",
	);
}

type RolloutScope = Pick<
	DeploymentRolloutStatus,
	"rollout_id" | "placement_id" | "project_id"
>;
function scopedRollout(
	value: unknown,
	scope: RolloutScope,
): DeploymentRolloutStatus {
	assertBoundedJson(value, 12_000);
	const status = rolloutSchema.parse(value);
	if (
		status.rollout_id !== scope.rollout_id ||
		status.placement_id !== scope.placement_id ||
		status.project_id !== scope.project_id
	)
		throw new Error(
			"The device returned rollout status for another project or placement.",
		);
	return status;
}
export async function readDeploymentRollout(
	call: ManagementCall,
	scope: RolloutScope,
): Promise<DeploymentRolloutStatus> {
	const response = await call({
		type: "rollout",
		rollout_id: scope.rollout_id,
	});
	const rejection = finalRejection(response);
	if (rejection)
		throw new DeploymentRejectedError(
			rejection,
			`reading the status of rollout ${scope.rollout_id}`,
		);
	if (response.state !== "completed")
		throw new Error(
			`The device could not confirm the status of rollout ${scope.rollout_id}.${rejectionDetail(response)} Reconnect and retry the same rollout.`,
		);
	return scopedRollout(response.result, scope);
}
export async function cancelDeploymentRollout(
	call: ManagementCall,
	scope: RolloutScope,
	signal?: AbortSignal,
): Promise<DeploymentRolloutStatus> {
	const current = await readDeploymentRollout(call, scope);
	signal?.throwIfAborted();
	if (!["staged", "validating"].includes(current.state)) return current;
	const id = crypto.randomUUID();
	const response = await call(
		{ type: "cancel_rollout", rollout_id: scope.rollout_id },
		id,
	);
	signal?.throwIfAborted();
	if (response.state === "rejected") {
		// A refusal after activation began is a race; the re-read status tells the caller what happened.
		const rejection = finalRejection(response);
		const status = await readDeploymentRollout(call, scope).catch(
			(error: unknown) => {
				if (rejection) return undefined;
				throw error;
			},
		);
		signal?.throwIfAborted();
		if (status && !["staged", "validating"].includes(status.state))
			return status;
		if (rejection)
			throw new DeploymentRejectedError(
				rejection,
				`discarding staged update ${scope.rollout_id}`,
			);
		throw new Error(
			`The device did not discard staged update ${scope.rollout_id}.${rejectionDetail(response)} Try again, or reload the placement to review it.`,
		);
	}
	if (
		response.operation_id !== id ||
		!["accepted", "completed"].includes(response.state)
	)
		throw new Error(
			"Discarding this update was not confirmed. Reconnect to read its durable status.",
		);
	const status = scopedRollout(response.result, scope);
	if (status.state !== "cancelled")
		throw new Error(
			"The device did not confirm that this staged update was discarded.",
		);
	return status;
}
async function rejectStep(
	call: ManagementCall,
	plan: DeploymentPlan,
	step: DeploymentPlan["steps"][number],
	terminal = false,
	rejection?: ManagementRejection,
): Promise<never> {
	const placement = String(plan.config.id);
	// Older agents give no reason, so a changed revision is the only proof of a stale read.
	const expected = Number(step.command.expected_revision);
	if (expected > 0) {
		const current = await readExistingDeployment(
			call,
			placement,
			String(plan.config.project_id),
		).catch(() => undefined);
		if (current && current.config_revision !== expected)
			throw new StaleDeploymentRevisionError(
				placement,
				expected,
				current.config_revision,
			);
	}
	const final = rejection && !rejection.retryable ? rejection : undefined;
	if (final && step.command.type === "apply")
		throw new DeploymentRejectedError(
			final,
			plan.expected_revision
				? `the update of placement ${placement}`
				: `creation of placement ${placement}`,
		);
	if (terminal || final)
		throw new DeploymentPublicationFailedError(
			placement,
			final
				? `The device rejected secret ${String(step.command.name)}: ${final.error}`
				: undefined,
		);
	throw new Error(
		step.command.type === "apply"
			? applyFailure(plan, rejection)
			: `The device rejected secret ${String(step.command.name)} for placement ${placement}.${rejection ? ` Device response: ${rejection.error}` : ""} Retry to confirm the same operations.`,
	);
}
export async function executeDeploymentPlan(
	call: ManagementCall,
	plan: DeploymentPlan,
	signal?: AbortSignal,
	onRolloutStatus?: (status: DeploymentRolloutStatus) => void,
): Promise<void> {
	if (plan.rollout_id) {
		await executeRolloutPlan(call, plan, signal, onRolloutStatus);
		await waitForDeploymentRollout(
			call,
			{
				rollout_id: plan.rollout_id,
				placement_id: String(plan.config.id),
				project_id: String(plan.config.project_id),
			},
			signal,
			onRolloutStatus,
		);
		return;
	}
	const expected = plan.expected_revision;
	for (const step of plan.steps) {
		signal?.throwIfAborted();
		// A retry first reads the journal. Reusing an operation ID with a new
		// timestamp must never repeat a command that the device already accepted.
		let response = step.attempted
			? await call({ type: "operation", operation_id: step.id })
			: undefined;
		signal?.throwIfAborted();
		if (!response || response.state === "rejected") {
			step.attempted = true;
			response = await call(step.command, step.id);
		}
		signal?.throwIfAborted();
		if (response.state === "rejected")
			await rejectStep(call, plan, step, false, managementRejection(response));
		if (response.operation_id !== step.id)
			throw new Error("The device returned a different deployment operation.");
		if (step.command.type === "apply") {
			const revision = response.result.config_revision;
			if (
				!["accepted", "completed"].includes(response.state) ||
				response.result.placement_id !== plan.config.id ||
				revision !== expected + 1
			)
				throw new Error(applyFailure(plan));
			continue;
		}
		const checkSecretBinding = (value: {
			operation_id: string;
			result: Record<string, unknown>;
		}) => {
			if (
				value.operation_id !== step.id ||
				value.result.placement_id !== step.command.placement_id ||
				value.result.name !== step.command.name
			)
				throw new Error(
					"Secret publication belongs to a different placement or secret.",
				);
		};
		checkSecretBinding(response);
		if (response.state === "failed") await rejectStep(call, plan, step, true);
		for (
			let attempts = 0;
			["pending", "accepted"].includes(response.state) && attempts < 20;
			attempts++
		) {
			await new Promise((resolve) => setTimeout(resolve, 250));
			signal?.throwIfAborted();
			response = await call({ type: "operation", operation_id: step.id });
			signal?.throwIfAborted();
			if (response.state === "rejected") await rejectStep(call, plan, step);
			checkSecretBinding(response);
			if (response.state === "failed") await rejectStep(call, plan, step, true);
		}
		checkSecretBinding(response);
		if (
			response.state !== "completed" ||
			response.result.secret !== "completed"
		)
			throw new Error(
				"Provisioning is incomplete. The placement remains stopped. Retry to confirm the same operations.",
			);
	}
}
