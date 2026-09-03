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

export interface IDatabaseDetail {
	readonly version?: string;
	readonly databaseName?: string;
	readonly largestTables: readonly ITableUsage[];
	readonly connections: readonly IConnectionStateCount[];
	readonly counters?: IDatabaseCounters;
	readonly rates?: IDatabaseRates;
	readonly statsResetAt?: string;
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
