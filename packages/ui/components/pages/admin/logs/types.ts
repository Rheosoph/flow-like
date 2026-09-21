import type {
	IAuditChainReport,
	IAuditEpochReport,
} from "../../../audit/types";

export interface IErrorReportRecord {
	id: string;
	user_id?: string | null;
	method: string;
	path: string;
	status_code: number;
	public_code: string;
	summary: string;
	details?: unknown;
	created_at: string;
	updated_at: string;
}

export interface IErrorBucket {
	key: string;
	label: string;
	count: number;
}

export interface IErrorStatsResponse {
	window_hours: number;
	total_errors: number;
	server_errors: number;
	client_errors: number;
	unique_users_affected: number;
	unique_paths: number;
	previous_window_total: number;
	change_percent?: number | null;
	recent: IErrorReportRecord[];
	top_codes: IErrorBucket[];
	top_paths: IErrorBucket[];
	top_users: IErrorBucket[];
}

export interface IErrorTimeseriesPoint {
	bucket: string;
	total: number;
	server: number;
	client: number;
}

export interface IErrorTimeseriesResponse {
	window_hours: number;
	bucket: string;
	points: IErrorTimeseriesPoint[];
}

export interface IListErrorsResponse {
	errors: IErrorReportRecord[];
	total: number;
	offset: number;
	limit: number;
}

export interface IRecentAuditChain {
	chain_id: string;
	latest_seal_seq?: number | null;
	latest_sealed_at_ms?: number | null;
	pending: number;
}

export interface ILatestAuditArchive {
	/** `YYYY-MM`. */
	period: string;
	created_at_ms: number;
	record_count: number;
}

/** `GET /admin/logs/chain-status`. */
export interface IChainStatusResponse {
	/** Key id of the serving process's signer, else the configured `AUDIT_KID`. */
	signing_kid?: string | null;
	verifying_kids: string[];
	epochs: IAuditEpochReport;
	latest_epoch_at_ms?: number | null;
	pending_records: number;
	/** Timestamp of the oldest record not yet sealed. */
	oldest_pending_ms?: number | null;
	quarantined_records: number;
	unanchored_seals: number;
	/** `sealedAt` of the oldest seal still waiting for its signed epoch; held chains excluded. */
	oldest_unanchored_ms?: number | null;
	/** Chains whose seal failed its hash or MAC check: never signed, they need an operator. */
	held_chains: number;
	total_records: number;
	total_seals: number;
	chains: number;
	latest_archive?: ILatestAuditArchive | null;
	/** Rows of the retired `AuditEntry` table still waiting for the one-time export. */
	legacy_entries: number;
	platform: IAuditChainReport;
	recent_chains: IRecentAuditChain[];
	/** `retention.pending_alert_seconds`. */
	pending_alert_seconds?: number | null;
	/** `retention.epoch_interval_seconds`: seals wait up to this long for their epoch. */
	epoch_interval_seconds?: number | null;
}

export function statusCodeTone(code: number) {
	if (code >= 500)
		return {
			variant: "destructive" as const,
			label: "Server",
			color: "text-destructive",
			ring: "border-destructive/40 bg-destructive/5",
		};
	if (code >= 400)
		return {
			variant: "secondary" as const,
			label: "Client",
			color: "text-amber-600 dark:text-amber-400",
			ring: "border-amber-500/40 bg-amber-500/5",
		};
	return {
		variant: "outline" as const,
		label: "Info",
		color: "text-muted-foreground",
		ring: "border-border bg-muted/30",
	};
}
