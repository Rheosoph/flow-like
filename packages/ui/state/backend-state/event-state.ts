import type {
	IEvent,
	IEventVariant,
	IIntercomEvent,
	ILogMetadata,
	IOAuthProvider,
	IOAuthToken,
	IRunPayload,
	IVersionType,
	PageTrigger,
} from "../../lib";
import type { IPrerunEventResponse } from "./types";

export interface IOAuthCheckResult {
	tokens?: Record<string, IOAuthToken>;
	missingProviders: IOAuthProvider[];
}

export interface IEventRegistration {
	id: string;
	event_id: string;
	event_version: string;
	kind: string;
	method?: string | null;
	path: string;
	node_id?: string | null;
	schema?: Record<string, any> | null;
	extras?: Record<string, any> | null;
	auth_id?: string | null;
}

export interface IEventRemoteAuth {
	id: string;
	event_id: string;
	event_version: string;
	kind: string;
	node_id: string;
	config: Record<string, any>;
}

export interface IListRegistrationsResponse {
	event_id: string;
	event_version?: string | null;
	/** The registration bucket listed: `stable` or a Live variant's name. */
	variant: string;
	registrations: IEventRegistration[];
	auths?: IEventRemoteAuth[];
}

export interface ISetupEventResponse {
	run_id: string;
	event_id: string;
	event_version: string;
	status: string;
	server_configs_received: number;
	registrations_written: number;
	auths_written: number;
	error?: string | null;
}

export interface IEventAlias {
	slug: string;
	app_id: string;
	event_id: string;
	created_by?: string | null;
}

/** One event revision on the timeline — the live head or an archived version. */
export interface IEventTimelineEntry {
	/** Event version as `[major, minor, patch]`. */
	version: [number, number, number];
	/**
	 * Dotted `"major.minor.patch"` — the same format the Lance runs store keeps
	 * in `event_version`, so runs group against entries by this key. Board
	 * versions on runs use the other format (`v{major}-{minor}-{patch}`).
	 */
	version_key: string;
	is_live: boolean;
	name: string;
	description: string;
	event_type: string;
	active: boolean;
	board_id?: string | null;
	board_version?: [number, number, number] | null;
	node_id?: string | null;
	default_page_id?: string | null;
	route?: string | null;
	is_default: boolean;
	execution_mode: string;
	exposure: string;
	created_at_ms: number;
	updated_at_ms: number;
	/** Whether the revision's target board still loads. */
	board_resolves: boolean;
	/** Whether the revision's target node still exists on that board. */
	node_resolves: boolean;
	/** Variable IDs only — values are never part of the timeline. */
	variable_ids: string[];
	secret_variable_ids: string[];
	notes_kind?: "notes" | "url" | null;
}

export interface IEventTimeline {
	event_id: string;
	/** Distinct board IDs across all entries, the live head's board first. */
	boards: string[];
	/** The archive listing hit the version cap; older entries are not shown. */
	truncated: boolean;
	/** Archived versions that were listed but could not be loaded. */
	skipped: number;
	/** Live head first, then archived versions newest-first. */
	entries: IEventTimelineEntry[];
}

/**
 * Run summary from the event runs listing: `ILogMetadata`-shaped, `payload`
 * always empty, `event_version` dotted like the timeline's `version_key`.
 */
export type IEventTimelineRun = ILogMetadata;

export interface IEventRunsResult {
	runs: IEventTimelineRun[];
	/** Boards whose run stores were successfully queried. */
	boards_queried: string[];
}

export type IRestoreIssueSeverity = "Blocking" | "Warning";

export type IRestoreIssueCode =
	| "BoardMissing"
	| "BoardVersionMissing"
	| "NodeMissing"
	| "PageMissing"
	| "EventTypeChanged"
	| "TargetKindChanged"
	| "FloatingBoard"
	| "SecretUnrecoverable"
	| "RouteConflict"
	| "CronScheduleUnchanged";

export interface IRestoreIssue {
	code: IRestoreIssueCode;
	severity: IRestoreIssueSeverity;
	message: string;
	subject: string | null;
}

/** One display-level field difference — `from`/`to` never carry secret values. */
export interface IRestoreFieldChange {
	field: string;
	from: string;
	to: string;
}

export interface IRestorePlan {
	/** The event as it would be persisted — secret variable values are blanked. */
	restored: IEvent;
	diff: IRestoreFieldChange[];
	/** Fields a restore never copies from the snapshot. */
	not_restored: string[];
	issues: IRestoreIssue[];
}

export interface IRestorePlanResult {
	plan: IRestorePlan;
	/** The event as persisted — present only after a non-dry run. */
	event?: IEvent;
	/** Outcome of the non-fatal REST/MCP re-setup after a non-dry run. */
	setup_status?: string | null;
}

export type IEventVariantStatsWindow = "24h" | "7d";

/** Rolling run aggregates for one dispatch target of an event. */
export interface IEventVariantStats {
	/** The `EventVariant.name` that served these runs; `null` for the primary. */
	variant_name: string | null;
	requests: number;
	errors: number;
	/** Microseconds, like every other run-duration surface. */
	p50_duration_us: number;
	p95_duration_us: number;
}

export interface IEventVariantStatsResult {
	/** The window the aggregates cover, echoed back (`24h` or `7d`). */
	window: string;
	variants: IEventVariantStats[];
}

/**
 * One traffic-share change. Weight/sample-rate edits go through the dedicated
 * PATCH route: they never cut an event version and never re-run setup.
 */
export interface IEventVariantSharePatch {
	name: string;
	/** Live variants: share of traffic replaced, `[0, 1]`. */
	weight?: number;
	/** Shadow variants: share of traffic mirrored, `[0, 1]`. */
	sample_rate?: number;
}

export interface ICanaryExplainResult {
	/** The live variant serving this key; `null` when the primary serves it. */
	variant_name: string | null;
	/** The `[lo, hi)` slice of the unit interval owned by this key's target. */
	share_bounds: [number, number];
}

/** One `EventSetup` pointer row: the registration bucket a variant serves. */
export interface IEventSetupInfo {
	/** `stable` for the primary target, else the `EventVariant.name`. */
	variant: string;
	/** The event version whose registration bucket this variant serves. */
	event_version: string;
	board_id: string;
	/** Dotted `major.minor.patch`; `null` floats on latest. */
	board_version?: string | null;
	/** `ok`, `running` or `error`. */
	setup_status?: string | null;
	/** UTC timestamp without timezone offset. */
	last_setup_at?: string | null;
	last_setup_error?: string | null;
}

export interface ICanaryPromoteResult {
	/** The event after the promote, secrets blanked. */
	event: IEvent;
	/**
	 * REST/MCP only: outcome of the non-fatal stable setup re-run — `ok`, or
	 * `{status}: {detail}` on failure. The promote holds either way; inbound
	 * serves the previous registration set until a setup succeeds.
	 */
	setup_status?: string | null;
	/**
	 * Regression-gate outcome for the promoted target, present when the
	 * event's suite gate is `Warn` or `Block` (a blocking `fail` is a 409, so
	 * a response carrying this field always went through).
	 */
	gate?: IPromotionGateSummary | null;
}

/* ------------------------------------------------ regression suites (Track D) */

export type IRegressionVerdict = "pass" | "fail" | "error";
export type IRegressionGateMode = "Off" | "Warn" | "Block";

/** The regression gate as surfaced on a canary promote response. */
export interface IPromotionGateSummary {
	gate_mode: string;
	/** `pass`, `fail` or `not_run`. */
	verdict: string;
	suite_run_id?: string | null;
	regressed?: number | null;
}

/**
 * One corpus candidate: a recorded real input of this event, eligible for
 * fixture promotion. Previews are redacted by leaf key name.
 */
export interface IRegressionCorpusEntry {
	run_id: string;
	/** Unix micros. */
	start: number;
	/** Unix micros. */
	end: number;
	/** The run's highest log level (3 = error, 4 = fatal). */
	log_level: number;
	/** Board version label as stored on the run row (`v{major}-{minor}-{patch}`). */
	board_version: string;
	event_version?: string | null;
	/** The node the run was dispatched into — the node a replay must target. */
	node_id: string;
	/** Raw recorded payload size in bytes, pre-redaction. */
	payload_len: number;
	/** Structural hash of the payload shape — values never enter it. */
	shape_hash: string;
	/** Redacted preview, capped at 2 KiB. */
	preview: string;
	/** Any of `rejected`, `too_large` and `empty`. */
	caveats: string[];
}

export interface IEventCorpusResult {
	/** Selected entries, newest first, failing inputs never selected away. */
	entries: IRegressionCorpusEntry[];
	board_id: string;
	/** The scan window the selection was drawn from, in seconds. */
	window_secs: number;
	scanned_rows: number;
	/** The scan hit its row cap; the selection is an arbitrary window subset. */
	scan_capped: boolean;
}

export interface IRegressionCorpusPayload {
	run_id: string;
	/** Resolve Re-Run against this, never against the run row's event id. */
	node_id: string;
	board_id: string;
	/** The recorded payload, redacted by leaf key name. */
	payload: unknown;
}

/** The verdict recorded at promotion — what replays are compared against. */
export interface IRegressionFixtureBaseline {
	verdict: IRegressionVerdict;
	error_class?: string | null;
	visited_node_ids?: string[];
	/** Unix micros — the recorded run's start. */
	recorded_at: number;
}

export interface IRegressionFixtureSummary {
	id: string;
	/** The node a replay of this fixture dispatches into. */
	source_node_id: string;
	source_board_id: string;
	baseline: IRegressionFixtureBaseline;
	promoted_by: string;
	/** Well-known values: `grading_blind`, `caller_oauth_tokens`. */
	caveats: string[];
}

/** Suite configuration — the bucket object, echoed by GET/PUT. */
export interface IRegressionSuiteConfig {
	id: string;
	board_id: string;
	event_id?: string | null;
	node_id: string;
	trigger_on_publish: boolean;
	schedule?: string | null;
	gate_mode: IRegressionGateMode;
	/**
	 * Must be acknowledged before the suite's first run: replay isolation only
	 * guards storage writes and WASM — outbound HTTP from native nodes still
	 * fires. Runs are refused while `false`.
	 */
	allow_live_side_effects: boolean;
	/** Unix micros. */
	created_at: number;
	/** Unix micros. */
	updated_at: number;
}

export interface IRegressionSuiteResult {
	suite: IRegressionSuiteConfig;
	/** Next scheduled run (RFC 3339), when a schedule is set. */
	next_run_at?: string | null;
	/** Promoted fixtures, without payloads. */
	fixtures: IRegressionFixtureSummary[];
}

export interface IPutRegressionSuiteRequest {
	trigger_on_publish: boolean;
	/** Cron expression; omit or null to clear the schedule. */
	schedule?: string | null;
	gate_mode?: IRegressionGateMode;
	allow_live_side_effects: boolean;
}

export interface IRegressionRunAccepted {
	/** Poll `getRegressionRun` with this id for progress. */
	suite_run_id: string;
	status: string;
}

export interface IRegressionSuiteRunSummary {
	id: string;
	/** Candidate board version (`major.minor.patch`, or `draft`). */
	board_version: string;
	/** `manual`, `publish` or `schedule`. */
	trigger: string;
	/** `running`, `completed` or `errored`. */
	status: string;
	regressed: number;
	fixed: number;
	still_failing: number;
	ok: number;
	skipped: number;
	/** RFC 3339. */
	started_at?: string | null;
	/** RFC 3339. */
	completed_at?: string | null;
	error?: string | null;
	/** RFC 3339. */
	created_at: string;
}

export interface IRegressionCaseResult {
	id: string;
	/** `recorded_fixture` or `authored_test`. */
	case_kind: string;
	/** Fixture id (recorded) or start node id (authored). */
	case_ref: string;
	/** The replay's execution run id; `null` when the case was skipped. */
	replay_run_id?: string | null;
	/** `ok`, `regressed`, `still_failing`, `fixed` or `skipped`. */
	outcome: string;
	/** Raw grader verdict of the replay: `pass`, `fail`, `error` or `skipped`. */
	grade_verdict: string;
	/** Diagnostics: error classes, failed assertions, grading-blind stamp, alias. */
	detail?: Record<string, unknown> | null;
	duration_ms?: number | null;
}

export interface IRegressionSuiteRunDetail {
	run: IRegressionSuiteRunSummary;
	cases: IRegressionCaseResult[];
}

export interface IEventState {
	/** Whether events always execute remotely (server-side). When true, secrets are handled server-side and don't need to be prompted or sent from the client. */
	readonly alwaysRemote?: boolean;

	getEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent>;
	getEvents(appId: string, force?: boolean): Promise<IEvent[]>;
	getEventVersions(
		appId: string,
		eventId: string,
	): Promise<[number, number, number][]>;
	upsertEvent(
		appId: string,
		event: IEvent,
		versionType?: IVersionType,
		personalAccessToken?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	): Promise<IEvent>;
	/** Check OAuth requirements for an event's board. Returns missing providers. */
	checkEventOAuth?(appId: string, event: IEvent): Promise<IOAuthCheckResult>;
	/** Check OAuth requirements resolved by a governed pre-run endpoint. */
	checkOAuthRequirements?(
		appId: string,
		requirements: Array<{ provider_id: string; scopes: string[] }>,
	): Promise<IOAuthCheckResult>;
	deleteEvent(appId: string, eventId: string): Promise<void>;
	validateEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<void>;
	upsertEventFeedback(
		appId: string,
		eventId: string,
		feedbackId: string,
		feedback: {
			rating: number;
			history?: any[];
			globalState?: Record<string, any>;
			localState?: Record<string, any>;
			comment?: string;
		},
	): Promise<string>;
	executeEvent(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined>;

	/** Execute an event remotely via the server-side SSE invoke endpoint */
	executeEventRemote?(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined>;

	cancelExecution(runId: string): Promise<void>;

	isEventSinkActive(eventId: string): Promise<boolean>;

	/**
	 * List persisted REST/MCP registrations for an event (populated by remote
	 * setup). `variant` selects a Live variant's registration bucket; omitted
	 * it lists the stable (primary) bucket.
	 */
	listEventRegistrations?(
		appId: string,
		eventId: string,
		version?: string,
		variant?: string,
	): Promise<IListRegistrationsResponse>;

	/**
	 * Run remote setup and persist REST/MCP registrations for an event.
	 * `variant` targets one Live variant's own registration bucket; omitted it
	 * sets up the stable (primary) target.
	 */
	setupEvent?(
		appId: string,
		eventId: string,
		force?: boolean,
		variant?: string,
	): Promise<ISetupEventResponse>;

	/** List vanity aliases for an event. */
	listEventAliases?(appId: string, eventId: string): Promise<IEventAlias[]>;

	/** Create or replace the event's vanity alias. */
	upsertEventAlias?(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<IEventAlias>;

	/** Delete a vanity alias from an event. */
	deleteEventAlias?(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<void>;

	/** Pre-run analysis: get required runtime variables and OAuth for an event */
	prerunEvent?(
		appId: string,
		eventId: string,
		version?: [number, number, number],
		pageTrigger?: PageTrigger,
	): Promise<IPrerunEventResponse>;

	/** Version history for an event: the live head plus archived versions, newest first. */
	getEventTimeline?(appId: string, eventId: string): Promise<IEventTimeline>;

	/**
	 * Run summaries for an event across the given boards (sourced from the
	 * timeline's board list), merged newest first.
	 */
	listEventRuns?(
		appId: string,
		eventId: string,
		boardIds: string[],
		options?: { limit?: number; offset?: number },
	): Promise<IEventTimelineRun[]>;

	/**
	 * Plan (dry run, the default) or apply a forward-only restore of an
	 * archived event version. Applying cuts a new version whose content
	 * matches the snapshot — never a rewind.
	 */
	restoreEvent?(
		appId: string,
		eventId: string,
		version: [number, number, number],
		options?: {
			dryRun?: boolean;
			versionType?: string;
			restoreRoute?: boolean;
			dropCanary?: boolean;
			acceptBlankSecrets?: boolean;
		},
	): Promise<IRestorePlanResult>;

	/** Per-variant request/error/latency aggregates for a window (cloud only). */
	getCanaryStats?(
		appId: string,
		eventId: string,
		window?: IEventVariantStatsWindow,
	): Promise<IEventVariantStatsResult>;

	/**
	 * Change one variant's weight/sample rate without cutting an event version
	 * or re-running setup — the slider path (cloud only).
	 */
	patchCanary?(
		appId: string,
		eventId: string,
		patch: IEventVariantSharePatch,
	): Promise<IEvent>;

	/** Replace the event's variant list (cloud only). */
	putEventVariants?(
		appId: string,
		eventId: string,
		variants: IEventVariant[],
	): Promise<IEvent>;

	/**
	 * Recompute which live variant a split key resolves to. Assignments are a
	 * pure hash, so past or hypothetical keys can be checked (cloud only).
	 */
	explainCanary?(
		appId: string,
		eventId: string,
		key: string,
		source?: string,
	): Promise<ICanaryExplainResult>;

	/**
	 * Promote a variant: its target becomes the event's primary, the variant
	 * is removed and a new event version is cut (cloud only).
	 */
	promoteCanary?(
		appId: string,
		eventId: string,
		variant: string,
		versionType?: IVersionType,
	): Promise<ICanaryPromoteResult>;

	/**
	 * Abort a variant: remove it so its traffic share returns to the primary
	 * immediately (cloud only).
	 */
	abortCanary?(
		appId: string,
		eventId: string,
		variant: string,
	): Promise<IEvent>;

	/** Per-variant REST/MCP setup health from the `EventSetup` rows (cloud only). */
	listEventSetups?(appId: string, eventId: string): Promise<IEventSetupInfo[]>;

	/**
	 * Recent real inputs recorded for this event, deduplicated by payload
	 * shape with failing inputs preserved — the regression-fixture candidates
	 * (cloud only). Previews are redacted.
	 */
	getEventCorpus?(
		appId: string,
		eventId: string,
		limit?: number,
	): Promise<IEventCorpusResult>;

	/**
	 * One recorded run's full (redacted) input payload plus the node it was
	 * dispatched into (cloud only).
	 */
	getCorpusPayload?(
		appId: string,
		eventId: string,
		runId: string,
	): Promise<IRegressionCorpusPayload>;

	/**
	 * Promote a recorded run into a regression fixture. The run is graded to
	 * capture the baseline verdict future replays are compared against;
	 * `acknowledgeRejected` must be set to promote a run that never executed
	 * (cloud only).
	 */
	promoteRegressionFixture?(
		appId: string,
		eventId: string,
		runId: string,
		options?: {
			expectation?: "pass" | "fail";
			acknowledgeRejected?: boolean;
		},
	): Promise<IRegressionFixtureSummary>;

	/** Delete a regression fixture and its stored payload (cloud only). */
	deleteRegressionFixture?(
		appId: string,
		eventId: string,
		fixtureId: string,
	): Promise<void>;

	/**
	 * The event's regression-suite configuration and its promoted fixtures;
	 * `null` when no suite has been saved yet (cloud only).
	 */
	getRegressionSuite?(
		appId: string,
		eventId: string,
	): Promise<IRegressionSuiteResult | null>;

	/**
	 * Create or update the event's regression suite. Scheduling a suite whose
	 * fixtures carry caller OAuth tokens is refused with a conflict
	 * (cloud only).
	 */
	putRegressionSuite?(
		appId: string,
		eventId: string,
		config: IPutRegressionSuiteRequest,
	): Promise<IRegressionSuiteResult>;

	/**
	 * Start a regression-suite run against a candidate board version —
	 * fire-and-forget: poll `getRegressionRun` with the returned id. Requires
	 * the suite's live-side-effects acknowledgement (cloud only).
	 */
	runRegressionSuite?(
		appId: string,
		eventId: string,
		options?: {
			boardVersion?: [number, number, number];
			allowDraft?: boolean;
		},
	): Promise<IRegressionRunAccepted>;

	/** The event's regression-suite runs, newest first (cloud only). */
	listRegressionRuns?(
		appId: string,
		eventId: string,
	): Promise<IRegressionSuiteRunSummary[]>;

	/** One regression-suite run with its per-case verdicts (cloud only). */
	getRegressionRun?(
		appId: string,
		eventId: string,
		suiteRunId: string,
	): Promise<IRegressionSuiteRunDetail>;
}
