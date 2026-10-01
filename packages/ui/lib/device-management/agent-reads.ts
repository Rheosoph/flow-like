import { z } from "zod";
import { type DeploymentRolloutStatus, rolloutSchema } from "./deployment";
import { hostOperationSchema } from "./inspection";
import type {
	AgentFeature,
	AgentFeatures,
	HostOperationView,
} from "./model/types";
import type { ManagementCall } from "./telemetry";
import { rejectionMessage } from "./transport";
import { managementRejection } from "./types";
import { LiveCallError, rejectionCode } from "./workspace/errors";

/**
 * Clients for the agent commands of plan §3.4.3. An agent without the matching
 * `features` flag is never asked; one that answers `rejected · unsupported`
 * reads the same way, so screens show "Update the device agent to see …".
 */
export type AgentRead<T> =
	| { kind: "ok"; data: T }
	| { kind: "unsupported"; feature: AgentFeature };

export function agentSupports(
	features: AgentFeatures | undefined,
	feature: AgentFeature,
): boolean {
	return features?.[feature] === 1;
}

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const managementId = z.string().regex(/^[A-Za-z0-9_:.-]{1,128}$/u);
const uuid = z.string().uuid();
const state = z.string().regex(/^[a-z_]{1,32}$/u);
const sha256Hex = z.string().regex(/^[a-f0-9]{64}$/u);
const text = (max: number) => z.string().max(max);

function firstIssue(error: unknown): string {
	if (error instanceof z.ZodError) {
		const issue = error.issues[0];
		return issue
			? `${issue.path.join(".") || "result"}: ${issue.message}`
			: "invalid";
	}
	return error instanceof Error ? error.message : String(error);
}

function limitWithin(
	value: number | undefined,
	fallback: number,
	max: number,
	what: string,
): number {
	const limit = value ?? fallback;
	if (!Number.isInteger(limit) || limit < 1 || limit > max)
		throw new RangeError(
			`${what} limit must be an integer from 1 to ${max}, got ${limit}.`,
		);
	return limit;
}

async function agentRead<T>(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	feature: AgentFeature,
	what: string,
	command: Record<string, unknown>,
	parse: (result: Record<string, unknown>) => T,
	operationId?: string,
): Promise<AgentRead<T>> {
	if (!agentSupports(features, feature))
		return { kind: "unsupported", feature };
	const response = await call(command, operationId);
	if (response.state === "rejected") {
		const rejection = managementRejection(response);
		if (rejection?.code === "unsupported")
			return { kind: "unsupported", feature };
		throw new LiveCallError(
			rejectionCode(rejection),
			rejectionMessage(response) ?? `The device refused to return the ${what}.`,
		);
	}
	if (
		response.state !== "completed" ||
		(operationId !== undefined && response.operation_id !== operationId)
	)
		throw new Error(
			`The device answered the ${what} request with state "${response.state}" instead of a result. Reconnect and retry.`,
		);
	try {
		return { kind: "ok", data: parse(response.result) };
	} catch (error) {
		throw new Error(
			`The device returned an invalid ${what} (${firstIssue(error)}).`,
		);
	}
}

/* BG15: the current host operation, whoever started it. */

export function readHostOperation(
	call: ManagementCall,
	features: AgentFeatures | undefined,
): Promise<AgentRead<HostOperationView | null>> {
	return agentRead(
		call,
		features,
		"host_operation",
		"current device operation",
		{ type: "host_operation" },
		(result) =>
			z.object({ operation: hostOperationSchema.nullable() }).parse(result)
				.operation,
	);
}

/* BG12: terminal and current rollouts of one placement, newest first. */

export interface RolloutHistoryPage {
	placement_id: string;
	rollouts: DeploymentRolloutStatus[];
	/** Pass as `before` for older rollouts. */
	next: string | null;
}

export function readRolloutHistory(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { placementId: string; before?: string; limit?: number },
): Promise<AgentRead<RolloutHistoryPage>> {
	const limit = limitWithin(input.limit, 8, 16, "Rollout history");
	return agentRead(
		call,
		features,
		"rollout_history",
		`rollout history of placement ${input.placementId}`,
		{
			type: "rollout_history",
			placement_id: input.placementId,
			...(input.before ? { before: input.before } : {}),
			limit,
		},
		(result) =>
			z
				.object({
					placement_id: z.literal(input.placementId),
					rollouts: z
						.array(
							rolloutSchema.refine(
								(row) => row.placement_id === input.placementId,
								"rollout of another placement",
							),
						)
						.max(limit),
					next: managementId.nullable(),
				})
				.parse(result),
	);
}

/* BG13: who did what in the last 24 h (owner only; `kind` is null until agent schema 13). */

export interface AgentOperation {
	operation_id: string;
	kind: string | null;
	actor: {
		role: "owner" | "grant";
		user_id: string | null;
		grant_id: string | null;
	};
	project_id: string | null;
	placement_id: string | null;
	accepted_at: number;
	state: "accepted" | "completed" | "failed" | "unknown" | (string & {});
}

const agentOperationSchema: z.ZodType<AgentOperation> = z.object({
	operation_id: managementId,
	kind: state.nullable(),
	actor: z.object({
		role: z.enum(["owner", "grant"]),
		user_id: managementId.nullable(),
		grant_id: managementId.nullable(),
	}),
	project_id: managementId.nullable(),
	placement_id: managementId.nullable(),
	accepted_at: count,
	state,
});

export interface AgentOperationsPage {
	operations: AgentOperation[];
	/** Pass as `after` for the next page. */
	next: string | null;
}

export function readOperations(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { after?: string; limit?: number } = {},
): Promise<AgentRead<AgentOperationsPage>> {
	const limit = limitWithin(input.limit, 20, 50, "Operations");
	return agentRead(
		call,
		features,
		"operations",
		"operation list",
		{
			type: "operations",
			...(input.after ? { after: input.after } : {}),
			limit,
		},
		(result) =>
			z
				.object({
					operations: z.array(agentOperationSchema).max(limit),
					next: managementId.nullable(),
				})
				.parse(result),
	);
}

/* BG16: projected metric history. Mirrors the agent's field allowlist. */

export const METRICS_HISTORY_FIELDS = {
	device: [
		"cpu_percent",
		"memory_used_bytes",
		"memory_total_bytes",
		"agent_cpu_percent_of_one_core",
		"agent_memory_bytes",
		"ready_replicas",
		"desired_replicas",
		"placements",
	],
	placement: [
		"cpu_percent",
		"memory_bytes",
		"running_replicas",
		"desired_replicas",
		"processes_observed",
	],
} as const;
export type DeviceMetricField = (typeof METRICS_HISTORY_FIELDS.device)[number];
export type PlacementMetricField =
	(typeof METRICS_HISTORY_FIELDS.placement)[number];

export type MetricsHistoryInput = (
	| { placementId: null; fields: readonly DeviceMetricField[] }
	| { placementId: string; fields: readonly PlacementMetricField[] }
) & { after?: number; limit?: number };

/** `[unix seconds, value per requested field]`; null where the sample had none. */
export type MetricPoint = [number, ...(number | null)[]];

export interface MetricsHistoryPage {
	fields: string[];
	points: MetricPoint[];
	/** Pass as `after` for newer points; null once caught up. */
	next: number | null;
	/** Records up to this number were evicted before they could be read. */
	evicted_through: number | null;
}

function metricFields(input: MetricsHistoryInput): string[] {
	const allowed: readonly string[] =
		input.placementId === null
			? METRICS_HISTORY_FIELDS.device
			: METRICS_HISTORY_FIELDS.placement;
	const fields = [...input.fields];
	if (
		!fields.length ||
		fields.length > 8 ||
		new Set(fields).size !== fields.length ||
		fields.some((field) => !allowed.includes(field))
	)
		throw new RangeError(
			`Metric history needs 1 to 8 distinct fields from [${allowed.join(", ")}], got [${fields.join(", ")}].`,
		);
	return fields;
}

export function readMetricsHistory(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: MetricsHistoryInput,
): Promise<AgentRead<MetricsHistoryPage>> {
	const fields = metricFields(input);
	const limit = limitWithin(input.limit, 256, 256, "Metric history");
	const after = input.after ?? 0;
	if (!Number.isSafeInteger(after) || after < 0)
		throw new RangeError(
			`Metric history cursor must be a non-negative integer, got ${after}.`,
		);
	const point = z
		.tuple([count])
		.rest(z.number().finite().nullable())
		.refine((row) => row.length === fields.length + 1, "point width");
	return agentRead(
		call,
		features,
		"metrics_history",
		`metric history of ${input.placementId ?? "the device"}`,
		{
			type: "metrics_history",
			placement_id: input.placementId,
			after,
			limit,
			fields,
		},
		(result) =>
			z
				.object({
					fields: z
						.array(z.string())
						.refine(
							(echo) => echo.join() === fields.join(),
							"fields differ from the request",
						),
					points: z.array(point).max(limit),
					next: count.nullable(),
					evicted_through: count.nullable(),
				})
				.parse(result) as MetricsHistoryPage,
	);
}

/* BG18: queued-write detail and tombstones. Never payloads. */

const queueScope = z.string().regex(/^[a-f0-9]{64}$/u);

export interface OfflineOperationSummary {
	sequence: number;
	operation_id: string;
	/** JSON of the offline resource (table or file path). */
	resource: string;
	mutation_kind: string | null;
	state: string;
	attempts: number;
	created_at: number;
	bytes: number;
	error: string | null;
	error_code: string | null;
}

export interface OfflineOperationLookup {
	/** Any state, including the applied, skipped and superseded tombstones. */
	state: string;
	superseded_by: string | null;
	error: string | null;
	error_code: string | null;
}

const offlineOperationSchema: z.ZodType<OfflineOperationSummary> = z.object({
	sequence: count,
	operation_id: uuid,
	resource: text(4096),
	mutation_kind: text(64).nullable(),
	state,
	attempts: count,
	created_at: z.number().int().safe(),
	bytes: count,
	error: text(2048).nullable(),
	error_code: text(64).nullable(),
});

const offlineLookupSchema: z.ZodType<OfflineOperationLookup> = z.object({
	state,
	superseded_by: uuid.nullable(),
	error: text(2048).nullable(),
	error_code: text(64).nullable(),
});

export interface OfflineOperationsPage {
	operations: OfflineOperationSummary[];
	/** Pass as `afterSequence` for the next page. */
	next: number | null;
}

export function readOfflineOperations(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: {
		placementId: string;
		scope: string;
		afterSequence?: number;
		limit?: number;
		/** Applied, skipped and superseded tombstones instead of open operations. */
		terminal?: boolean;
	},
): Promise<AgentRead<OfflineOperationsPage>> {
	const limit = limitWithin(input.limit, 20, 50, "Queued write");
	queueScope.parse(input.scope);
	return agentRead(
		call,
		features,
		"offline_lookup",
		`queued writes of placement ${input.placementId}`,
		{
			type: "offline_queue_operations",
			placement_id: input.placementId,
			scope: input.scope,
			...(input.afterSequence === undefined
				? {}
				: { after_sequence: input.afterSequence }),
			limit,
			terminal: input.terminal ?? false,
		},
		(result) =>
			z
				.object({
					operations: z.array(offlineOperationSchema).max(limit),
					next: count.nullable(),
				})
				.parse(result),
	);
}

export function lookupOfflineOperation(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { placementId: string; scope: string; queuedOperationId: string },
): Promise<AgentRead<OfflineOperationLookup>> {
	queueScope.parse(input.scope);
	uuid.parse(input.queuedOperationId);
	return agentRead(
		call,
		features,
		"offline_lookup",
		`queued write ${input.queuedOperationId}`,
		{
			type: "offline_queue_lookup",
			placement_id: input.placementId,
			scope: input.scope,
			queued_operation_id: input.queuedOperationId,
		},
		(result) => offlineLookupSchema.parse(result),
	);
}

/* BG17: artifact capacity and explicit revision cleanup. */

const quota = z.object({ used: count, max: count.nullable() });
const budget = z.object({ bytes: quota, entries: quota, revisions: quota });

export type ArtifactQuota = z.infer<typeof quota>;
export type ArtifactBudget = z.infer<typeof budget>;
export interface ArtifactRevisionUsage {
	revision: string;
	bytes: number;
	/** Placements whose configuration pins this revision. */
	referenced_by: string[];
	/** An active rollout's previous or candidate revision. */
	rollout: boolean;
}
export interface ArtifactUsage {
	/** Null without device-scope Deploy or ownership. */
	device: ArtifactBudget | null;
	/** Null without Deploy on the project, or when no project was asked for. */
	project: ArtifactBudget | null;
	revisions: ArtifactRevisionUsage[];
}
export interface ArtifactPruneResult {
	project_id: string;
	pruned: string[];
	freed_bytes: number;
}

const artifactUsageSchema: z.ZodType<ArtifactUsage> = z.object({
	device: budget.nullable(),
	project: budget.nullable(),
	revisions: z
		.array(
			z.object({
				revision: sha256Hex,
				bytes: count,
				referenced_by: z.array(managementId).max(256),
				rollout: z.boolean(),
			}),
		)
		.max(1024),
});

function projectId(value: string): string {
	if (
		!/^[A-Za-z0-9_.-]{1,128}$/u.test(value) ||
		value === "." ||
		value === ".."
	)
		throw new RangeError(`Invalid project identifier "${value}".`);
	return value;
}

export function readArtifactUsage(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { projectId?: string | null } = {},
): Promise<AgentRead<ArtifactUsage>> {
	const project = input.projectId ? projectId(input.projectId) : null;
	return agentRead(
		call,
		features,
		"artifact_capacity",
		"app storage usage",
		{ type: "artifact", request: { kind: "usage", project_id: project } },
		(result) => artifactUsageSchema.parse(result),
	);
}

/** Deletes exactly the listed revisions; the device refuses referenced ones and any while uploads run (`busy`). */
export function pruneArtifactRevisions(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { projectId: string; revisions: string[] },
): Promise<AgentRead<ArtifactPruneResult>> {
	const project = projectId(input.projectId);
	const revisions = [...new Set(input.revisions)];
	if (
		!revisions.length ||
		revisions.length > 64 ||
		revisions.some((revision) => !sha256Hex.safeParse(revision).success)
	)
		throw new RangeError(
			`Choose 1 to 64 app revisions (SHA-256 hex) to remove, got ${input.revisions.length}.`,
		);
	return agentRead(
		call,
		features,
		"artifact_capacity",
		"revision cleanup result",
		{
			type: "artifact",
			request: { kind: "prune", project_id: project, revisions },
		},
		(result) =>
			z
				.object({
					project_id: z.literal(project),
					pruned: z
						.array(sha256Hex)
						.refine(
							(pruned) => pruned.every((value) => revisions.includes(value)),
							"removed a revision that was not requested",
						),
					freed_bytes: count,
				})
				.parse(result),
		crypto.randomUUID(),
	);
}
