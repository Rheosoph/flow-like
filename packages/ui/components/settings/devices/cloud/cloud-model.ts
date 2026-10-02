import type {
	EffectiveLimit,
	OnlineAccess,
	ResourceSummary,
} from "../../../../lib/device-management/model/types";
import type {
	BillingGrant,
	DeviceResources,
	InstanceLease,
	ResourceGrant,
} from "../../../../lib/device-resources";

export type ApprovalState = "active" | "revoked" | "expired";

/** One spending limit as the screens show it; amounts in micro-euros. */
export interface CloudLimit {
	id: string;
	grantId: string;
	limit: number;
	used: number;
	reserved: number;
	expiresAt: number;
	/** `active` still accepts charges. */
	state: ApprovalState;
	payerIsMe: boolean;
	/** Known from the device's own list only. */
	payerId?: string;
	version?: number;
	createdAt?: number;
}

/** What only the device owner and the approver can read. */
export interface ApprovalDetails {
	models: string[];
	maxInstances: number;
	version: number;
	projectId: string;
	deploymentId: string;
	/** BG36; absent on older hubs. */
	approvedBy?: string;
	approvedAt?: number;
}

export interface CloudApproval {
	deviceId: string;
	serviceId: string;
	grantId: string;
	/** The online app; null for a local-only app (model access). */
	appId: string | null;
	state: ApprovalState;
	/**
	 * Not revoked and its approved date isn't reached: it can still be revoked,
	 * and the hub accepts no second approval for the service until it is. An
	 * approval that ended early (BG32) stays revocable; it resumes when what
	 * ended it comes back.
	 */
	revocable: boolean;
	/** The date the approval was given for. */
	expiresAt: number;
	/** BG32: when it really ends and what ends it; absent on older hubs and on revoked approvals. */
	effective?: { at: number; limit: EffectiveLimit };
	files: OnlineAccess | "none";
	/** BG33: true only when the hub says so (absent also means unknown). */
	writeBlocked: boolean;
	approverIsMe: boolean;
	details?: ApprovalDetails;
	limit?: CloudLimit;
	/** Instances holding a lease; undefined when the device's list wasn't read. */
	leases?: InstanceLease[];
}

/** When the approval stops (or stopped) giving out credentials. */
export const endOf = (
	approval: Pick<CloudApproval, "effective" | "expiresAt">,
) => approval.effective?.at ?? approval.expiresAt;

function stateOf(
	status: "active" | "revoked",
	end: number,
	now: number,
): ApprovalState {
	if (status === "revoked") return "revoked";
	return end > now ? "active" : "expired";
}

function limitOfGrant(
	billing: readonly BillingGrant[],
	grantId: string,
	me: string,
	now: number,
): CloudLimit | undefined {
	const own = billing.filter((row) => row.grant_id === grantId);
	const current =
		own.find((row) => row.status === "active" && row.expires_at > now) ??
		own[0];
	if (!current) return undefined;
	return {
		id: current.billing_grant_id,
		grantId,
		limit: current.limit_micros,
		used: current.used_micros,
		reserved: current.reserved_micros,
		expiresAt: current.expires_at,
		state: stateOf(current.status, current.expires_at, now),
		payerIsMe: current.payer_id === me,
		payerId: current.payer_id,
		version: current.authz_version,
		...(current.created_at === undefined
			? {}
			: { createdAt: current.created_at }),
	};
}

function fromGrant(
	grant: ResourceGrant,
	resources: DeviceResources,
	me: string,
	now: number,
): CloudApproval {
	const effective =
		grant.status === "active" && grant.effective_expires_at !== undefined
			? {
					at: grant.effective_expires_at,
					limit: grant.effective_limit ?? ("approval" as const),
				}
			: undefined;
	const limit = limitOfGrant(resources.billing, grant.grant_id, me, now);
	return {
		deviceId: grant.device_id,
		serviceId: grant.placement_id,
		grantId: grant.grant_id,
		appId: grant.app_id,
		state: stateOf(grant.status, effective?.at ?? grant.expires_at, now),
		revocable: grant.status === "active" && grant.expires_at > now,
		expiresAt: grant.expires_at,
		...(effective ? { effective } : {}),
		files: grant.online_access ?? "none",
		writeBlocked: grant.online_write_blocked === "storage_full",
		approverIsMe: grant.delegating_user_id === me,
		details: {
			models: grant.model_ids,
			maxInstances: grant.max_instances,
			version: grant.authz_version,
			projectId: grant.project_id,
			deploymentId: grant.deployment_id,
			approvedBy: grant.approved_by_user_id ?? grant.delegating_user_id,
			...(grant.created_at === undefined
				? {}
				: { approvedAt: grant.created_at }),
		},
		...(limit ? { limit } : {}),
		leases: resources.instances.filter(
			(lease) => lease.grant_id === grant.grant_id,
		),
	};
}

/** The device's own lists: every approval the viewer may read in full. */
export function approvalsOfDevice(
	resources: DeviceResources,
	me: string,
	now: number,
): CloudApproval[] {
	return resources.grants.map((grant) => fromGrant(grant, resources, me, now));
}

type SummaryDevice = ResourceSummary["devices"][number];

/** The summary never lists a withdrawn limit, so one that isn't running has run out. */
function limitOfSummary(
	billing: SummaryDevice["billing"][number],
	now: number,
): CloudLimit {
	return {
		id: billing.billing_grant_id,
		grantId: billing.grant_id,
		limit: billing.limit_micros,
		used: billing.used_micros,
		reserved: billing.reserved_micros,
		expiresAt: billing.expires_at,
		state: stateOf("active", billing.expires_at, now),
		payerIsMe: billing.payer_is_me,
	};
}

function fromSummary(device: SummaryDevice, now: number): CloudApproval[] {
	return device.approvals.map((row) => {
		const billing = device.billing.find(
			(entry) => entry.grant_id === row.grant_id,
		);
		// A revoked row carries parser defaults, not a real end (E19).
		const effective =
			row.status === "active"
				? { at: row.effective_expires_at, limit: row.effective_limit }
				: undefined;
		const approval: CloudApproval = {
			deviceId: device.device_id,
			serviceId: row.placement_id,
			grantId: row.grant_id,
			appId: row.app_id,
			state: stateOf(row.status, row.effective_expires_at, now),
			revocable: row.status === "active" && row.expires_at > now,
			expiresAt: row.expires_at,
			files: row.online_access ?? "none",
			writeBlocked: row.online_write_blocked === "storage_full",
			approverIsMe: row.approver_is_me,
		};
		if (effective) approval.effective = effective;
		if (billing) approval.limit = limitOfSummary(billing, now);
		return approval;
	});
}

/** The fleet-wide list (E19), without the details only a device's own list carries. */
export function approvalsOfSummary(
	summary: ResourceSummary,
	now: number,
): CloudApproval[] {
	return summary.devices.flatMap((device) => fromSummary(device, now));
}

/** A summary row completed with what the device's own list knows about the same approval. */
function merged(summary: CloudApproval, own: CloudApproval): CloudApproval {
	const limit = own.limit ?? summary.limit;
	return {
		...summary,
		...own,
		writeBlocked: own.writeBlocked || summary.writeBlocked,
		...(limit ? { limit } : {}),
	};
}

/**
 * Fleet rows first (hub order), each completed from its device's list; then
 * approvals only a device's list knows (older hubs, or read since the summary).
 */
export function mergeApprovals(
	summary: readonly CloudApproval[],
	perDevice: readonly CloudApproval[],
): CloudApproval[] {
	const own = new Map(perDevice.map((row) => [row.grantId, row]));
	const listed = new Set(summary.map((row) => row.grantId));
	return [
		...summary.map((row) => {
			const detail = own.get(row.grantId);
			return detail ? merged(row, detail) : row;
		}),
		...perDevice.filter((row) => !listed.has(row.grantId)),
	];
}

const RANK: Record<ApprovalState, number> = {
	active: 0,
	expired: 1,
	revoked: 2,
};

/** Running approvals first; the order inside a state is the hub's. */
export function byState(rows: readonly CloudApproval[]): CloudApproval[] {
	return rows
		.map((row, index) => ({ row, index }))
		.sort((a, b) => RANK[a.row.state] - RANK[b.row.state] || a.index - b.index)
		.map(({ row }) => row);
}

/** The approval a service runs on: the active one, else the one that ended last. */
export function currentApproval(
	rows: readonly CloudApproval[],
	serviceId: string,
): CloudApproval | undefined {
	const own = rows.filter((row) => row.serviceId === serviceId);
	return (
		own.find((row) => row.state === "active") ??
		[...own].sort((a, b) => endOf(b) - endOf(a))[0]
	);
}

export interface SpendTotals {
	count: number;
	used: number;
	reserved: number;
	limit: number;
}

/** "You pay for N spending limits": limits of the viewer that still accept charges. */
export function spendTotals(rows: readonly CloudApproval[]): SpendTotals {
	const totals: SpendTotals = { count: 0, used: 0, reserved: 0, limit: 0 };
	for (const { limit } of rows) {
		if (!limit?.payerIsMe || limit.state !== "active") continue;
		totals.count += 1;
		totals.used += limit.used;
		totals.reserved += limit.reserved;
		totals.limit += limit.limit;
	}
	return totals;
}

/** Whole days from `now` to `at`, at least 1 (the form's "Ends after" field). */
export function daysUntil(at: number, now: number): number {
	return Math.max(1, Math.round((at - now) / 86_400));
}
