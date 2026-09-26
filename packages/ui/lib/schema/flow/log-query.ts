/**
 * Structured filter over one run's logs. The backend turns it into an escaped
 * storage predicate, so no caller ever writes filter syntax. Levels use the
 * stored numbers: 0 Debug, 1 Info, 2 Warn, 3 Error, 4 Fatal. Times are
 * microseconds since the Unix epoch. Rows always come back oldest first.
 */
export interface ILogQuery {
	levels?: number[];
	exclude_levels?: number[];
	nodes?: string[];
	exclude_nodes?: string[];
	/** Every phrase must occur in the message, case-insensitively. */
	text?: string[];
	/** No phrase may occur in the message, case-insensitively. */
	exclude_text?: string[];
	fingerprints?: string[];
	exclude_fingerprints?: string[];
	from?: number;
	to?: number;
	/** Each listed group keeps only its first occurrence. */
	fold?: ILogFold[];
}

export interface ILogFold {
	fingerprint: string;
	first_start: number;
}

/**
 * Counts and repeat groups of one run, recorded while it ran. Runs recorded
 * before fingerprints get a summary computed from their log table:
 * `fingerprinted` is false there, so folding and "hide similar" are unavailable.
 */
export interface IRunLogSummary {
	version: number;
	fingerprinted: boolean;
	/** The legacy scan stopped at its row cap; counts are lower bounds. */
	partial: boolean;
	total: number;
	/** Per level: [debug, info, warn, error, fatal]. */
	levels: number[];
	/** Node id ("" for logs without a node) → per-level counts. */
	nodes: Record<string, number[]>;
	first_error?: ISummaryLog | null;
	/** Nodes that executed in this run, logged or not; `null` when unknown. */
	visited?: string[] | null;
	/** Most frequent groups first. */
	groups: ILogGroup[];
	groups_truncated: boolean;
}

export interface ISummaryLog {
	node_id?: null | string;
	log_level: number;
	start: number;
	message: string;
	fingerprint?: null | string;
}

export interface ILogGroup {
	fingerprint: string;
	node_id?: null | string;
	log_level: number;
	/** The message with `⟨n⟩` for numbers and `⟨id⟩` for hex ids and UUIDs. */
	template: string;
	/** One entry per `⟨n⟩` in `template`, in order: the smallest and largest value seen. */
	slots: Array<ILogSlotRange | null>;
	count: number;
	first_start: number;
	last_start: number;
	/** The first message of the group, verbatim (truncated). */
	sample: string;
}

export interface ILogSlotRange {
	min: number;
	max: number;
}
