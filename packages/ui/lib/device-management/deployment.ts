import { z } from "zod";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IEventState } from "../../state/backend-state/event-state";
import type { IEvent } from "../schema/flow/event";
import type { ProjectArtifactAssets } from "./artifacts";
import type { ManagementCall } from "./telemetry";

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
const eventSchema = z.object({
	id: identifier,
	name: z.string().max(480),
	event_type: z.string().max(128),
	event_version: version.nullable(),
	board_version: version.nullable(),
	hosted: z.boolean(),
	readiness_kind: z.enum(["listener", "explicit", "unsupported"]).optional(),
	rollout_supported: z.boolean().optional(),
	eligible: z.boolean(),
});
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
		auth_secret: identifier,
	})
	.passthrough();
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
			(config.max_replicas === 1 || config.hosting != null)
		);
	});
export type DeploymentEvent = z.infer<typeof eventSchema>;
export type DeploymentVariable = z.infer<typeof variableSchema>;
const rolloutSchema = z.object({
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
});
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
	constructor(placementId: string) {
		super(
			`Secret publication failed for placement ${placementId}. Reload this existing placement before preparing another update.`,
		);
		this.name = "DeploymentPublicationFailedError";
	}
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
			`The device did not return the configuration of placement ${placementId}. Updating it requires deploy access to this placement.`,
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
export type InstalledProject = {
	project_id: string;
	project_path: string;
	revision?: string;
	source: "offline" | "online";
	assets?: ProjectArtifactAssets;
};
export type DeploymentCatalog = {
	events: DeploymentEvent[];
	variables: Record<string, DeploymentVariable[]>;
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
				"The device could not read this published project revision.",
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
export async function discoverOnlineEvents(
	events: IEventState,
	project: string,
): Promise<DeploymentEvent[]> {
	const rows = await events.getEventsAuthoritative(project);
	if (rows.length > 512)
		throw new Error("Project inventory exceeds its discovery limit.");
	return rows
		.map(onlineEvent)
		.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

function onlineEvent(event: IEvent): DeploymentEvent {
	const hosted =
		Boolean(event.default_page_id) ||
		["http", "simple_chat"].includes(event.event_type);
	const eventVersion = version.safeParse(event.event_version);
	const boardVersion = version.safeParse(event.board_version);
	return eventSchema.parse({
		id: event.id,
		name: event.name.slice(0, 120),
		event_type: event.event_type,
		event_version: eventVersion.success ? eventVersion.data : null,
		board_version: boardVersion.success ? boardVersion.data : null,
		hosted,
		readiness_kind:
			hosted || ["rest", "mcp"].includes(event.event_type)
				? "listener"
				: event.event_type === "daemon"
					? "explicit"
					: "unsupported",
		rollout_supported:
			hosted || ["rest", "mcp", "daemon"].includes(event.event_type),
		eligible:
			event.active &&
			!event.canary &&
			!event.variants?.length &&
			eventVersion.success &&
			boardVersion.success &&
			(hosted || ["daemon", "rest", "mcp"].includes(event.event_type)),
	});
}
export async function discoverOnlineVariables(
	events: IEventState,
	boards: IBoardState,
	project: string,
	selected: DeploymentEvent,
): Promise<DeploymentVariable[]> {
	if (!selected.eligible || !selected.event_version || !selected.board_version)
		throw new Error("Choose a published event with a concrete board version.");
	// The picker selects the current definition; it may not have an archive yet.
	const event = await events.getEventAuthoritative(project, selected.id);
	const current = onlineEvent(event);
	if (
		current.id !== selected.id ||
		!current.eligible ||
		current.event_type !== selected.event_type ||
		current.hosted !== selected.hosted ||
		JSON.stringify(current.event_version) !==
			JSON.stringify(selected.event_version) ||
		JSON.stringify(current.board_version) !==
			JSON.stringify(selected.board_version)
	)
		throw new Error(
			"Published event pins or eligibility changed. Reload the project.",
		);
	const board = await boards.getBoardAuthoritative(
		project,
		event.board_id,
		selected.board_version,
	);
	if (
		board.id !== event.board_id ||
		JSON.stringify(board.version) !== JSON.stringify(selected.board_version)
	)
		throw new Error("Published board pins differ.");
	return mergeVariables([
		Object.values(board.variables)
			.concat(
				...Object.values(board.layers).map((layer) =>
					Object.values(layer.variables),
				),
			)
			.filter((v) => v.exposed || v.runtime_configured)
			.map((v) =>
				variableSchema.parse({
					id: v.id,
					name: v.name.slice(0, 120),
					data_type: v.data_type,
					value_type: v.value_type,
					secret: v.secret,
				}),
			),
	]);
}
export function mergeVariables(
	groups: DeploymentVariable[][],
): DeploymentVariable[] {
	const rows = new Map<string, DeploymentVariable>();
	for (const variable of groups.flat()) {
		const previous = rows.get(variable.id);
		if (previous && JSON.stringify(previous) !== JSON.stringify(variable))
			throw new Error(
				"These events disagree about a variable. Deploy them separately.",
			);
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
	overrides: Record<string, string>;
	host: string;
	port: number;
	replicas: number;
	serviceToken: string;
	resourceGrant?: Record<string, unknown>;
	/** Undefined preserves existing settings; null explicitly disables buffering. */
	offlineWrites?: OfflineWritesConfig | null;
	/** Undefined preserves existing limits; null selects the trusted process profile. */
	resourceLimits?: PlacementResources | null;
};
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
	const definitions = new Map(
		input.variables.map((variable) => [variable.id, variable]),
	);
	for (const [id, text] of Object.entries(input.overrides)) {
		const definition = definitions.get(id);
		if (!definition)
			throw new Error("A variable is outside the selected events.");
		// Empty secret inputs mean keep the opaque reference, never read or replace it.
		if (definition.secret && text === "") continue;
		const value = variableValue(definition, text);
		if (definition.secret) {
			const name = `variable-${crypto.randomUUID()}`;
			delete plain[id];
			references[id] = name;
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
	return { plain, references, secrets };
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
	const resourceGrant = existing
		? (existing.config.resource_grant ?? undefined)
		: input.resourceGrant;
	if (installed.source === "online" && !resourceGrant)
		throw new Error("Select a resource approval for the online project.");
	const hosted = events.some((event) => event.hosted);
	if (
		!Number.isInteger(input.replicas) ||
		input.replicas < 1 ||
		input.replicas > 32 ||
		(input.replicas > 1 && events.some((event) => !event.hosted))
	)
		throw new Error(
			"Only HTTP, chat and Page services can use multiple replicas.",
		);
	const previousHosting = existing?.config.hosting ?? undefined;
	const replaceToken =
		hosted && (!previousHosting || input.serviceToken !== "");
	if (
		hosted &&
		(!z.string().ip().safeParse(input.host).success ||
			!Number.isInteger(input.port) ||
			input.port < 1 ||
			input.port > 65535 ||
			(replaceToken && !/^[\x21-\x7e]{32,4096}$/.test(input.serviceToken)))
	)
		throw new Error(
			"Set a listener IP, a port from 1 to 65535, and a service token of at least 32 printable characters.",
		);
	const { plain, references, secrets } = placementVariables(input);
	// A rotated token gets a fresh name so the running revision keeps its current token.
	const authSecret = !previousHosting
		? "service-access"
		: replaceToken
			? `service-access-${crypto.randomUUID()}`
			: previousHosting.auth_secret;
	if (replaceToken)
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
						...previousHosting,
						host: input.host,
						port: input.port,
						max_in_flight: previousHosting?.max_in_flight ?? 64,
						request_timeout_secs: previousHosting?.request_timeout_secs ?? 300,
						auth_secret: authSecret,
					},
				}
			: {}),
		...(resourceGrant ? { resource_grant: resourceGrant } : {}),
	};
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
	assertBoundedJson(config, 12_000);
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
	if (
		steps.some(
			(step) =>
				new TextEncoder().encode(JSON.stringify(step.command)).length > 12_000,
		)
	)
		throw new Error(
			"This deployment exceeds the management message limit. Use smaller placements.",
		);
	return { config, steps, expected_revision: expected, rollout_id: rolloutId };
}
function applyFailure(plan: DeploymentPlan): string {
	return plan.expected_revision
		? `The update of placement ${String(plan.config.id)} was not confirmed. Reload the placement and try again.`
		: "Placement creation was not confirmed. Its identity may already exist.";
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
			if (step.command.type === "stage_rollout") {
				const current = await readExistingDeployment(
					call,
					String(plan.config.id),
					String(plan.config.project_id),
				);
				if (current.config_revision !== plan.expected_revision)
					throw new StaleDeploymentRevisionError(
						String(plan.config.id),
						plan.expected_revision,
						current.config_revision,
					);
				if (current.desired_state === "stopped")
					throw new DeploymentReviewRequiredError(
						"The placement was stopped after this update was prepared. Reload it and review the update before continuing.",
					);
				if (
					current.rollout &&
					current.rollout.rollout_id !== plan.rollout_id &&
					["staged", "validating", "activating", "rolling_back"].includes(
						current.rollout.state,
					)
				)
					throw new DeploymentReviewRequiredError(
						"Another update is already in progress for this placement. Reload it to review that rollout.",
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
			throw new Error(
				"The device rejected this rollout operation. Retry to confirm its journal and current status.",
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
	if (response.state !== "completed")
		throw new Error(
			"The device could not confirm rollout status. Reconnect and retry the same rollout.",
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
	if (response.state === "rejected") return readDeploymentRollout(call, scope);
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
): Promise<never> {
	const placement = String(plan.config.id);
	// Rejections carry no reason, so a changed revision is the only proof of a stale read.
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
	if (terminal) throw new DeploymentPublicationFailedError(placement);
	throw new Error(
		step.command.type === "apply"
			? applyFailure(plan)
			: `The device rejected secret ${String(step.command.name)} for placement ${placement}. Retry to confirm the same operations.`,
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
		if (response.state === "rejected") await rejectStep(call, plan, step);
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
