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

export interface IChainSummary {
	chain_id?: string | null;
	label: string;
	entries: number;
	last_sequence?: number | null;
	last_entry_at?: string | null;
	last_entry_hash?: string | null;
	signed: boolean;
	kid?: string | null;
	valid?: boolean | null;
}

export interface IChainStatusResponse {
	signing_configured: boolean;
	current_kid: string;
	total_entries: number;
	signed_entries: number;
	unsigned_entries: number;
	branch_chain_count: number;
	last_24h_entries: number;
	root_chain: IChainSummary;
	recent_branches: IChainSummary[];
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
