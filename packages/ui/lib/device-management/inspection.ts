import { z } from "zod";
import type {
	InspectionPlus,
	PlacementStatusPlus,
	ReplicaStatusPlus,
} from "./model/types";
import type { ManagementCall } from "./telemetry";
import {
	type PlacementStatus,
	agentFeatures,
	managementRejection,
} from "./types";

function identifier(value: unknown): value is string {
	return typeof value === "string" && /^[A-Za-z0-9_:.-]{1,128}$/u.test(value);
}
function counter(value: unknown): value is number {
	return Number.isSafeInteger(value) && Number(value) >= 0;
}
export function placement(value: unknown): value is PlacementStatus {
	if (!value || typeof value !== "object") return false;
	const row = value as PlacementStatus;
	return (
		identifier(row.id) &&
		identifier(row.project_id) &&
		identifier(row.deployment_id) &&
		typeof row.revision === "string" &&
		row.revision.length <= 256 &&
		typeof row.desired_state === "string" &&
		typeof row.observed_state === "string" &&
		counter(row.config_revision) &&
		counter(row.intent_revision) &&
		(row.applied_revision === null || counter(row.applied_revision)) &&
		counter(row.desired_replicas) &&
		counter(row.running_replicas) &&
		counter(row.ready_replicas) &&
		counter(row.max_replicas) &&
		row.max_replicas >= 1 &&
		row.max_replicas <= 32 &&
		row.desired_replicas >= 1 &&
		row.desired_replicas <= row.max_replicas &&
		Array.isArray(row.replicas) &&
		row.replicas.length <= 32 &&
		row.replicas.every(
			(replica) =>
				counter(replica.slot) &&
				replica.slot < 32 &&
				typeof replica.observed_state === "string" &&
				(replica.applied_revision === null ||
					counter(replica.applied_revision)),
		) &&
		new Set(row.replicas.map((replica) => replica.slot)).size ===
			row.replicas.length
	);
}

/* CA4 agent facts (plan §3.4.2). Each is optional: a malformed value is dropped, never fatal. */

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const text = (max: number) => z.string().max(max);
const id = z.string().regex(/^[A-Za-z0-9_:.-]{1,128}$/u);
const version = z.tuple([count, count, count]);

const restarts = z.object({
	failures: count,
	max_restarts: count,
	crash_looping: z.boolean(),
	retry_in_seconds: count.nullish().transform((value) => value ?? null),
	last_started_at: count.nullish().transform((value) => value ?? null),
});

const PLACEMENT_FACTS = {
	process_id: count.nullable(),
	last_error: text(1024).nullable(),
	has_error: z.boolean(),
	restarts,
	offline_writes: z.object({
		scopes: count,
		pending_count: count,
		pending_bytes: count,
		oldest_at: count.nullable(),
		quarantined_scopes: count,
		needs_attention: count,
		mirror_error: z.boolean(),
	}),
	source: z.enum(["offline", "online"]),
	events: z
		.array(
			z.object({
				event_id: id,
				event_version: version,
				board_version: version,
			}),
		)
		.max(64),
	events_truncated: z.boolean(),
	online_metadata_sha256: z
		.string()
		.regex(/^[a-f0-9]{64}$/u)
		.nullable(),
} satisfies Partial<
	Record<keyof PlacementStatusPlus, z.ZodType<unknown, z.ZodTypeDef, unknown>>
>;

const REPLICA_FACTS = {
	process_id: PLACEMENT_FACTS.process_id,
	last_error: PLACEMENT_FACTS.last_error,
	has_error: PLACEMENT_FACTS.has_error,
	restarts,
} satisfies Partial<
	Record<keyof ReplicaStatusPlus, z.ZodType<unknown, z.ZodTypeDef, unknown>>
>;

/** BG15 host operation, shared with the `HostOperation` command reader (DM/agent-reads.ts). */
export const hostOperationSchema = z.object({
	operation_id: id,
	kind: z.enum(["reboot", "update_agent"]),
	state: z.string().regex(/^[a-z_]{1,32}$/u),
	created_at: count,
	issued_by: z.enum(["you", "owner", "another_person"]),
});

const task = z.object({
	name: z.string().regex(/^[a-z][a-z0-9_]{0,63}$/u),
	state: z.enum(["ok", "failing", "stopped"]),
	since: count,
	/** Absent in status snapshots: it changes on every failed pass. */
	consecutive_failures: count.optional(),
	category: z
		.enum(["hub_unreachable", "hub_refused", "storage", "policy", "internal"])
		.nullish(),
});

const DEVICE_FACTS = {
	agent_version: text(64),
	host_operations: z.object({
		reboot: z.boolean(),
		update_agent: z.boolean(),
	}),
	host_isolation: z.enum(["required", "optional", "none"]).nullable(),
	isolation: z
		.object({
			platform: text(64),
			sandbox_available: z.boolean(),
			require_isolation: z.boolean(),
			placement_preflight_required: z.boolean().optional(),
			network_boundary: text(256).optional(),
			disk_requirement: text(256).optional(),
			landlock_abi: z.number().int().nullish(),
			reason: text(1024).nullish(),
		})
		.nullable(),
	agent: z.object({
		version: text(64),
		release_version: text(64).nullable(),
		release_sequence: count.nullable(),
	}),
	host: z.object({
		booted_at: count.nullable(),
		agent_started_at: count,
	}),
	tasks: z.array(task).max(64),
	host_operation: hostOperationSchema.nullable(),
	network: z.object({
		interfaces: z
			.array(
				z.object({
					name: text(64),
					addresses: z.array(text(64)).max(32),
					loopback: z.boolean(),
				}),
			)
			.max(64),
	}),
};

type Facts = Record<string, z.ZodType<unknown, z.ZodTypeDef, unknown>>;

/** The fact keys present on `source` whose values are valid. */
function validFacts(source: unknown, facts: Facts): Record<string, unknown> {
	const output: Record<string, unknown> = {};
	if (!source || typeof source !== "object") return output;
	const row = source as Record<string, unknown>;
	for (const [key, schema] of Object.entries(facts)) {
		if (!(key in row)) continue;
		const parsed = schema.safeParse(row[key]);
		if (parsed.success) output[key] = parsed.data;
	}
	return output;
}

/** Copies `row`; every fact key present is kept when valid and removed otherwise. */
function withFacts<T extends object>(row: T, facts: Facts): T {
	const output: Record<string, unknown> = {
		...(row as Record<string, unknown>),
	};
	for (const key of Object.keys(facts)) delete output[key];
	return { ...output, ...validFacts(row, facts) } as T;
}

function placementPlus(row: PlacementStatus): PlacementStatusPlus {
	const output = withFacts(row, PLACEMENT_FACTS) as PlacementStatusPlus;
	if (!row.replicas) return output;
	return {
		...output,
		replicas: row.replicas.map((replica) =>
			withFacts(replica as ReplicaStatusPlus, REPLICA_FACTS),
		),
	};
}

function deviceFacts(
	page: Record<string, unknown>,
): Omit<InspectionPlus, "device_id" | "boot_id" | "placements" | "features"> {
	const facts = withFacts(page, DEVICE_FACTS);
	const pick = <K extends keyof typeof DEVICE_FACTS>(key: K) =>
		facts[key] as z.output<(typeof DEVICE_FACTS)[K]> | undefined;
	const output: Omit<
		InspectionPlus,
		"device_id" | "boot_id" | "placements" | "features"
	> = {};
	const assign = <K extends keyof typeof output>(
		key: K,
		value: (typeof output)[K] | undefined,
	) => {
		if (value !== undefined) output[key] = value;
	};
	assign("agentVersion", pick("agent_version"));
	assign("hostOperations", pick("host_operations"));
	assign("hostIsolation", pick("host_isolation"));
	assign("isolation", pick("isolation"));
	assign("agent", pick("agent"));
	assign("host", pick("host"));
	assign("tasks", pick("tasks"));
	assign("hostOperation", pick("host_operation"));
	assign("network", pick("network"));
	return output;
}

/* Encrypted status snapshots (plan §3.4.2): rows shed detail to stay small and never carry error text, process ids or retry times. */

const SNAPSHOT_PLACEMENT_FACTS = {
	has_error: PLACEMENT_FACTS.has_error,
	offline_writes: PLACEMENT_FACTS.offline_writes,
	source: PLACEMENT_FACTS.source,
	events: PLACEMENT_FACTS.events,
	events_truncated: PLACEMENT_FACTS.events_truncated,
	online_metadata_sha256: PLACEMENT_FACTS.online_metadata_sha256,
};
const SNAPSHOT_REPLICA_FACTS = {
	has_error: PLACEMENT_FACTS.has_error,
	restarts,
};
const SNAPSHOT_DEVICE_FACTS = {
	agent: DEVICE_FACTS.agent,
	host: DEVICE_FACTS.host,
	tasks: DEVICE_FACTS.tasks,
	host_operation: DEVICE_FACTS.host_operation,
};

/** Device-scope facts of a status snapshot: only unhealthy tasks, without their failure counter. */
export type SnapshotDeviceFacts = Pick<
	InspectionPlus,
	"agent" | "host" | "tasks" | "hostOperation"
>;

export function snapshotDeviceFacts(source: unknown): SnapshotDeviceFacts {
	const { host_operation, ...facts } = validFacts(
		source,
		SNAPSHOT_DEVICE_FACTS,
	);
	return {
		...facts,
		...(host_operation === undefined ? {} : { hostOperation: host_operation }),
	} as SnapshotDeviceFacts;
}

/** `base` is the retained row; of everything else `source` carries, only the validated snapshot facts are added. */
export function snapshotPlacement(
	base: PlacementStatus,
	source: unknown,
): PlacementStatusPlus {
	const replicas = (source as Partial<PlacementStatus> | null)?.replicas;
	return {
		...base,
		...validFacts(source, SNAPSHOT_PLACEMENT_FACTS),
		replicas: base.replicas?.map((replica) => ({
			...replica,
			...validFacts(
				Array.isArray(replicas)
					? replicas.find((row) => row?.slot === replica.slot)
					: undefined,
				SNAPSHOT_REPLICA_FACTS,
			),
		})),
	};
}

function certificateFacts(page: Record<string, unknown>) {
	const canDelegate = page.can_delegate_certificate_renewal === true;
	return {
		...(page.certificate_management === 1
			? {
					certificate_management: 1 as const,
					can_manage_certificates: page.can_manage_certificates === true,
				}
			: {}),
		...(page.certificate_issuance === 1
			? {
					certificate_issuance: 1 as const,
					can_delegate_certificate_renewal: canDelegate,
				}
			: {}),
		...(page.certificate_acme === 1
			? {
					certificate_acme: 1 as const,
					can_delegate_certificate_renewal: canDelegate,
				}
			: {}),
	};
}

export interface InspectionReadOptions {
	/** After every page: pages read and the expected total (`null` while unknown). */
	onPage?: (done: number, approxTotal: number | null) => void;
	/** Placement count of the previous read, used for `approxTotal`. */
	expectedPlacements?: number;
	/** Milliseconds for `observed_at`; defaults to `Date.now`. */
	now?: () => number;
}

function approxTotal(
	done: number,
	finished: boolean,
	expectedPlacements?: number,
): number | null {
	if (finished) return done;
	if (expectedPlacements === undefined) return null;
	return Math.max(done + 1, Math.ceil(expectedPlacements / 2));
}

interface InspectionPage {
	device_id: string;
	boot_id: string | null;
	placements: unknown[];
	next: string | null;
	[key: string]: unknown;
}

async function readPage(
	call: ManagementCall,
	after: string | null,
	expectedDevice: string,
): Promise<InspectionPage> {
	const response = await call({ type: "inspect_page", after, limit: 2 });
	if (response.state !== "completed") {
		const rejection = managementRejection(response);
		throw new Error(
			`This controller cannot read device placement status.${rejection ? ` ${rejection.error}` : ""}`,
		);
	}
	const result = response.result;
	if (
		result.device_id !== expectedDevice ||
		(result.boot_id !== null && !identifier(result.boot_id)) ||
		!Array.isArray(result.placements) ||
		result.placements.length > 2 ||
		(result.next !== null && !identifier(result.next))
	)
		throw new Error("Invalid device inspection page.");
	return result as InspectionPage;
}

/** Appends the page's rows in strict id order; returns the new cursor. */
function appendRows(
	rows: unknown[],
	after: string | null,
	placements: PlacementStatusPlus[],
): string | null {
	let cursor = after;
	for (const row of rows) {
		if (!placement(row) || (cursor !== null && row.id <= cursor))
			throw new Error("Device inspection pages overlap or changed order.");
		placements.push(placementPlus(row));
		cursor = row.id;
	}
	return cursor;
}

/** Each page fits a management frame even when every placement has 32 slots. */
export async function readDeviceInspection(
	call: ManagementCall,
	expectedDevice: string,
	options: InspectionReadOptions = {},
): Promise<InspectionPlus> {
	const placements: PlacementStatusPlus[] = [];
	const first = await readPage(call, null, expectedDevice);
	let result = first;
	let after: string | null = null;
	for (let page = 1; page <= 512; page++) {
		if (page > 1) {
			result = await readPage(call, after, expectedDevice);
			if (result.boot_id !== first.boot_id)
				throw new Error(
					"The device rebooted while status was being read. Refresh its status.",
				);
		}
		after = appendRows(result.placements, after, placements);
		const finished = result.next === null;
		options.onPage?.(
			page,
			approxTotal(page, finished, options.expectedPlacements),
		);
		if (finished)
			return {
				device_id: expectedDevice,
				boot_id: first.boot_id,
				placements,
				observed_at: (options.now ?? Date.now)(),
				features: agentFeatures(first.features),
				...certificateFacts(first),
				...deviceFacts(first),
			};
		if (!result.placements.length || result.next !== after)
			throw new Error("Device inspection returned an invalid continuation.");
	}
	throw new Error(
		"Device inspection exceeded 512 pages. Narrow the device inventory before retrying.",
	);
}
