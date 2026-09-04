/**
 * Mirror of `packages/api/src/routes/admin/resources/types.rs`.
 *
 * The API serializes with `rename_all = "camelCase"`, so field names are camelCase
 * while metric `key` values stay snake_case — those are data, not field names.
 */

export type IResourceKind = "database" | "cache" | "storage" | "stateStore";

export type IResourceHealth =
	| "ok"
	| "degraded"
	| "unavailable"
	| "unsupported"
	| "notConfigured";

export type IMetricUnit =
	| "bytes"
	| "count"
	| "milliseconds"
	| "seconds"
	| "ratio"
	| "perSecond";

/**
 * How current a value is. A provider's daily storage rollup and a live query must
 * never render identically.
 */
export type IMetricFreshness = "live" | "estimate" | "provider" | "rate";

export interface IResourceMetric {
	/** Stable identifier, e.g. `size_bytes`. Key off this, never the label. */
	readonly key: string;
	readonly label: string;
	readonly value: number;
	readonly unit: IMetricUnit;
	readonly freshness: IMetricFreshness;
	/** When the value was measured, for anything that is not `live`. */
	readonly observedAt?: string;
	/** Caveat to show next to the value, e.g. that a count is capped. */
	readonly note?: string;
}

export interface IResourceStatus {
	/** `database`, `cache`, `state-store`, `storage:meta`, `storage:content`, `storage:cdn`. */
	readonly id: string;
	readonly kind: IResourceKind;
	readonly label: string;
	/** Implementation in use: `postgres`, `redis`, `dynamodb`, `s3`, … */
	readonly backend: string;
	/** Which instance this is — bucket name, region, account. */
	readonly detail?: string;
	readonly status: IResourceHealth;
	readonly message?: string;
	readonly latencyMs?: number;
	readonly metrics: readonly IResourceMetric[];
}

export interface ITableUsage {
	readonly name: string;
	readonly totalBytes: number;
	readonly tableBytes: number;
	readonly indexBytes: number;
	/** Planner estimate, not a `COUNT(*)`. */
	readonly estimatedRows: number;
	readonly deadRows: number;
}

export interface IConnectionStateCount {
	readonly state: string;
	readonly count: number;
}

export interface IDatabaseCounters {
	readonly commits: number;
	readonly rollbacks: number;
	readonly tuplesReturned: number;
	readonly tuplesFetched: number;
	readonly tuplesInserted: number;
	readonly tuplesUpdated: number;
	readonly tuplesDeleted: number;
	readonly blocksHit: number;
	readonly blocksRead: number;
	readonly deadlocks: number;
	readonly tempFiles: number;
	readonly tempBytes: number;
}

export interface IDatabaseRates {
	readonly windowSeconds: number;
	readonly commits: number;
	readonly rollbacks: number;
	readonly tuplesRead: number;
	readonly tuplesWritten: number;
	readonly blocksRead: number;
}

/** Asynchronous schema jobs on engines that build indexes out of band. */
export interface IDatabaseJobs {
	readonly pending: number;
	readonly failed: number;
	readonly completed: number;
}

/** An index the planner ignores because its build failed or has not finished. */
export interface IInvalidIndex {
	readonly table: string;
	readonly name: string;
}

export interface IDatabaseDetail {
	readonly version?: string;
	readonly databaseName?: string;
	readonly largestTables: readonly ITableUsage[];
	readonly connections: readonly IConnectionStateCount[];
	readonly counters?: IDatabaseCounters;
	readonly rates?: IDatabaseRates;
	readonly statsResetAt?: string;
	/**
	 * Statistics this engine cannot provide at all (e.g. `size on disk`). The
	 * sections they feed come back empty, and without this the page shows them
	 * as blank cards that read like a failed query.
	 */
	readonly unsupported?: readonly string[];
	readonly jobs?: IDatabaseJobs;
	readonly invalidIndexes?: readonly IInvalidIndex[];
}

/** Server-side wording of {@link IDatabaseDetail.unsupported} entries. */
export const UNSUPPORTED_SIZE_ON_DISK = "size on disk";
export const UNSUPPORTED_TABLE_SIZES = "table sizes";
export const UNSUPPORTED_CONNECTION_STATES = "connection states";
export const UNSUPPORTED_ACTIVITY_COUNTERS = "activity counters";
export const UNSUPPORTED_DEAD_ROWS = "dead rows";

export function isUnsupported(
	detail: Pick<IDatabaseDetail, "unsupported">,
	...statistics: readonly string[]
): boolean {
	const reported = detail.unsupported;
	if (!reported || reported.length === 0) return false;
	return statistics.some((statistic) => reported.includes(statistic));
}

export interface IAdminResourcesResponse {
	readonly generatedAt: string;
	readonly cached: boolean;
	readonly resources: readonly IResourceStatus[];
	readonly databaseDetail?: IDatabaseDetail;
}

export const RESOURCES_ENDPOINT = "admin/resources";

/** Tailwind tone per health state, shared by the widget and the detail page. */
export function healthTone(status: IResourceHealth): {
	dot: string;
	badge: string;
} {
	switch (status) {
		case "ok":
			return {
				dot: "bg-emerald-500",
				badge: "border-emerald-500/30 text-emerald-600 dark:text-emerald-400",
			};
		case "degraded":
			return {
				dot: "bg-amber-500",
				badge: "border-amber-500/30 text-amber-600 dark:text-amber-400",
			};
		case "unavailable":
			return {
				dot: "bg-destructive",
				badge: "border-destructive/30 text-destructive",
			};
		default:
			return { dot: "bg-muted-foreground/40", badge: "border-border" };
	}
}

/**
 * A resource that is merely statistics-free or unconfigured is healthy. Only a
 * failed probe is a fault — painting the others red trains operators to ignore red.
 */
export function isFault(status: IResourceHealth): boolean {
	return status === "unavailable" || status === "degraded";
}
