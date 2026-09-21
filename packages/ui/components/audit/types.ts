export const PLATFORM_CHAIN = "platform";
export const ACTIVITY_SUFFIX = "#activity";

export type AuditRecordStatus = "pending" | "sealed" | "invalid";

export interface IAuditRecordView {
	id: string;
	chain_id: string;
	timestamp_ms: number;
	actor_id: string;
	actor_type: string;
	action: string;
	resource_type: string;
	resource_id: string;
	details?: unknown;
	actor_ip?: string | null;
	status: AuditRecordStatus;
	seal_id?: string | null;
	/** A commitment remains but the value expired under the retention policy. */
	ip_redacted: boolean;
	details_redacted: boolean;
}

export interface IAuditRecordCursor {
	before_ms: number;
	before_id: string;
}

export interface IAuditRecordPage {
	records: IAuditRecordView[];
	next?: IAuditRecordCursor | null;
}

export interface IAuditRecordFilters {
	/** A trailing `*` matches a prefix. */
	action?: string;
	actor_id?: string;
	resource_type?: string;
	resource_id?: string;
}

export interface IAuditChainReport {
	chain_id: string;
	valid: boolean;
	first_broken_seal?: number | null;
	problem?: string | null;
	seals_checked: number;
	/** First seal this run checked; earlier seals were verified by a previous run of this server process, or pruned. */
	checked_from_seq: number;
	records_checked: number;
	redacted_values: number;
	unanchored_seals: number;
	unverifiable_epochs: number;
	pending_records: number;
	pending_invalid: number;
	pruned_before_seq?: number | null;
	latest_seal_seq?: number | null;
	latest_epoch_seq?: number | null;
	empty: boolean;
	/** A seal failed its hash or MAC check: the chain is never signed into an epoch until an operator acts. */
	held: boolean;
}

export interface IAuditEpochReport {
	valid: boolean;
	first_broken_epoch?: number | null;
	problem?: string | null;
	epochs_checked: number;
	unverifiable_epochs: number;
	pruned_before_seq?: number | null;
	latest_epoch_seq?: number | null;
	latest_epoch_hash?: string | null;
}

export interface IAuditSealLine {
	id: string;
	chain_id: string;
	seq: number;
	class: string;
	prev_hash: string;
	record_count: number;
	records_root: string;
	first_at_ms: number;
	last_at_ms: number;
	sealed_at_ms: number;
	hash: string;
	epoch_seq?: number | null;
	epoch_index?: number | null;
	epoch_proof: string[];
}

export interface IAuditEpochLine {
	seq: number;
	prev_hash: string;
	seal_count: number;
	seals_root: string;
	created_at_ms: number;
	kid: string;
	signature: string;
	hash: string;
}

export interface IAuditHead {
	chain_id: string;
	seal?: IAuditSealLine | null;
	epoch?: IAuditEpochLine | null;
}

export type AuditHeadCheck = "matches" | "differs" | "archived";

export interface IAuditHeadCheckRequest {
	/** Epoch sequence. */
	seq: number;
	/** Epoch hash. */
	hash: string;
	/** Sent with `seal_seq` and `seal_hash` when the saved head carries its seal. */
	chain_id?: string;
	seal_seq?: number;
	seal_hash?: string;
}

export interface IAuditHeadCheckResponse {
	result: AuditHeadCheck;
}

export type AuditExportClass = "evidence" | "activity";

export interface IAuditWebhook {
	url: string;
	active: boolean;
	failures: number;
	last_error?: string | null;
	last_delivered_at_ms?: number | null;
	next_attempt_at_ms?: number | null;
	evidence_cursor: number;
	activity_cursor: number;
	created_at_ms: number;
}

export interface IAuditWebhookSaved extends IAuditWebhook {
	/** Only present when this call created the target. */
	secret?: string | null;
}

export interface IAuditWebhookInput {
	url: string;
	active?: boolean;
}

export interface IAuditSecret {
	secret: string;
}

export function activityChainOf(chainId: string): string {
	return `${chainId}${ACTIVITY_SUFFIX}`;
}
