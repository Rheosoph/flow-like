export interface ITelemetryTopEvent {
	name: string;
	count: number;
	installs: number;
}

export interface ITelemetrySourceBucket {
	source: string;
	count: number;
}

export interface ITelemetryPlatformBucket {
	platform: string;
	count: number;
}

export interface ITelemetryVersionBucket {
	appVersion: string;
	count: number;
}

export interface ITelemetryCountryBucket {
	country: string;
	count: number;
}

export interface ITelemetryOverviewResponse {
	hours: number;
	totalEvents: number;
	activeInstalls: number;
	previousTotalEvents: number;
	topEvents: ITelemetryTopEvent[];
	sources: ITelemetrySourceBucket[];
	platforms: ITelemetryPlatformBucket[];
	versions: ITelemetryVersionBucket[];
	countries: ITelemetryCountryBucket[];
}

export interface ITelemetryTimeseriesPoint {
	ts: string;
	count: number;
	installs: number;
}

export interface ITelemetryTimeseriesResponse {
	bucket: "minute" | "hour" | "day";
	points: ITelemetryTimeseriesPoint[];
}

export interface ITelemetryEventRow {
	id: string;
	name: string;
	source: string;
	anonId: string;
	props?: Record<string, unknown> | null;
	appVersion?: string | null;
	platform?: string | null;
	clientTs?: string | null;
	createdAt: string;
}

export interface ITelemetryEventsResponse {
	events: ITelemetryEventRow[];
	total: number;
	page: number;
	pageSize: number;
}

export interface ITelemetryDauPoint {
	ts: string;
	installs: number;
}

export interface ITelemetryRetentionCohort {
	cohortWeek: string;
	cohortSize: number;
	weeks: number[];
}

export interface ITelemetryDropOffPath {
	path: string;
	count: number;
}

export interface ITelemetryEngagementResponse {
	days: number;
	dau: ITelemetryDauPoint[];
	wau: number;
	mau: number;
	previousWau: number;
	previousMau: number;
	newInstalls: number;
	returningInstalls: number;
	churnedInstalls: number;
	churnRate: number | null;
	retention: ITelemetryRetentionCohort[];
	dropOffPaths: ITelemetryDropOffPath[];
}

export interface ITelemetryFlowpilotTotals {
	runsStarted: number;
	runsSucceeded: number;
	runsFailed: number;
	runsCancelled: number;
	plansAssessed: number;
	plansFeasible: number;
	plansInfeasible: number;
	attemptsTotal: number;
	attemptsParseValid: number;
	attemptsTypedValid: number;
	attemptsReconcileValid: number;
	attemptsApplied: number;
	queuedReviews: number;
	applyDispositions: number;
	dismissedDispositions: number;
	staleDispositions: number;
	errorDispositions: number;
	diagnosticOccurrences: number;
	repeatedDiagnosticOccurrences: number;
	validationRegressions: number;
	boardsInspected: number;
	emptyBoardsAfterRun: number;
	failuresTotal: number;
	subagentDispatchFailures: number;
	flowscriptApplyFailures: number;
	widgetApplyFailures: number;
	dataApplyFailures: number;
	pageApplyFailures: number;
	toolFailures: number;
	runFailures: number;
}

export type ITelemetryFlowpilotFailureKind =
	| "subagent_dispatch"
	| "flowscript_apply"
	| "widget_apply"
	| "data_apply"
	| "page_apply"
	| "tool_error"
	| "run_error";

export interface ITelemetryFlowpilotFailure {
	kind: ITelemetryFlowpilotFailureKind;
	tool: string;
	code: string;
	message: string;
	count: number;
	installs: number;
}

export interface ITelemetryFlowpilotTrendPoint {
	ts: string;
	runsStarted: number;
	runsSucceeded: number;
	runsFailed: number;
}

export interface ITelemetryFlowpilotResponse {
	hours: number;
	installs: number;
	totals: ITelemetryFlowpilotTotals;
	trend: ITelemetryFlowpilotTrendPoint[];
	failures: ITelemetryFlowpilotFailure[];
}

export type ITelemetryIssueStatus = "unresolved" | "resolved" | "ignored";

export interface ITelemetryIssue {
	id: string;
	fingerprint: string;
	kind: string;
	title: string;
	culprit?: string | null;
	level: string;
	source: string;
	platform?: string | null;
	status: string;
	firstSeen: string;
	lastSeen: string;
	eventCount: number;
	installCount: number;
	firstRelease?: string | null;
	lastRelease?: string | null;
}

export interface ITelemetryIssuesResponse {
	issues: ITelemetryIssue[];
	total: number;
	page: number;
	pageSize: number;
}

export interface ITelemetryIssueFrame {
	function?: string | null;
	file?: string | null;
	lineno?: number | null;
	colno?: number | null;
	in_app?: boolean | null;
}

export interface ITelemetryIssueBreadcrumb {
	ts?: string | null;
	category?: string | null;
	message?: string | null;
	level?: string | null;
}

export interface ITelemetryIssueEvent {
	id: string;
	anonId: string;
	source: string;
	platform?: string | null;
	appVersion?: string | null;
	release?: string | null;
	stacktrace?: ITelemetryIssueFrame[] | null;
	breadcrumbs?: ITelemetryIssueBreadcrumb[] | null;
	context?: Record<string, unknown> | null;
	country?: string | null;
	clientTs?: string | null;
	createdAt: string;
	symbolicated: boolean;
}

export interface ITelemetryIssuePoint {
	ts: string;
	count: number;
}

export interface ITelemetryReleaseBucket {
	release: string;
	count: number;
}

export interface ITelemetryIssueDetail {
	issue: ITelemetryIssue;
	latestEvent?: ITelemetryIssueEvent | null;
	timeseries: ITelemetryIssuePoint[];
	releases: ITelemetryReleaseBucket[];
	platforms: ITelemetryPlatformBucket[];
}

export interface ITelemetryReleaseRow {
	version: string;
	source: string;
	commitSha?: string | null;
	firstSeenAt: string;
	installs: number;
	sessions: number;
	crashedSessions: number;
	crashFreeSessionRate?: number | null;
	crashFreeInstallRate?: number | null;
	errorCount: number;
	adoption?: number | null;
}

export interface ITelemetryReleasesResponse {
	releases: ITelemetryReleaseRow[];
}

export interface ITelemetryReleaseHealthPoint {
	ts: string;
	sessions: number;
	crashedSessions: number;
	crashFreeSessionRate?: number | null;
}

export interface ITelemetryReleaseHealthResponse {
	hours: number;
	totalSessions: number;
	crashedSessions: number;
	crashFreeSessionRate?: number | null;
	crashFreeInstallRate?: number | null;
	totalInstalls: number;
	trend: ITelemetryReleaseHealthPoint[];
	releases: ITelemetryReleaseRow[];
}

export type ITelemetrySpanStatus = "ok" | "error";

export type ITelemetrySpanKind =
	| "server"
	| "client"
	| "internal"
	| "producer"
	| "consumer";

export interface ITelemetryTraceSummary {
	traceId: string;
	rootName: string;
	source: string;
	startedAt: string;
	durationMs: number;
	spanCount: number;
	status: string;
}

export interface ITelemetryTracesResponse {
	traces: ITelemetryTraceSummary[];
	total: number;
	page: number;
	pageSize: number;
}

export interface ITelemetryTraceSpan {
	id: string;
	spanId: string;
	parentSpanId?: string | null;
	name: string;
	kind: string;
	source: string;
	startedAt: string;
	durationMs: number;
	status: string;
	attributes?: Record<string, unknown> | null;
}

export interface ITelemetryTraceDetail {
	traceId: string;
	spans: ITelemetryTraceSpan[];
	rootName: string;
	totalDurationMs: number;
	spanCount: number;
}

export type ITelemetryPerfMetricName =
	| "lcp"
	| "inp"
	| "cls"
	| "ttfb"
	| "fcp"
	| "app_start"
	| "screen_load";

export type ITelemetryPerfRating = "good" | "needs-improvement" | "poor";

export interface ITelemetryPerfMetricSummary {
	metric: string;
	p50: number;
	p75: number;
	p95: number;
	count: number;
	rating: string;
}

export interface ITelemetryPerfTrendPoint {
	ts: string;
	metric: string;
	p75: number;
}

export interface ITelemetryPerfPathRow {
	path: string;
	metric: string;
	p75: number;
	count: number;
}

export interface ITelemetryPerformanceResponse {
	hours: number;
	metrics: ITelemetryPerfMetricSummary[];
	trend: ITelemetryPerfTrendPoint[];
	byPath: ITelemetryPerfPathRow[];
}

export interface ITelemetrySpanOperation {
	name: string;
	count: number;
	p50: number;
	p95: number;
	errorRate: number;
	totalMs: number;
}

export interface ITelemetrySpanStatsResponse {
	operations: ITelemetrySpanOperation[];
}

/** One rated FlowPilot assistant turn. Mirrors `PromptFeedbackRecord`. */
export interface IPromptFeedbackRecord {
	id: string;
	messageId: string;
	conversationId?: string | null;
	userId?: string | null;
	/** 5 for a positive rating, 1 for a negative one. */
	rating: number;
	comment: string;
	provider?: string | null;
	model?: string | null;
	reasoningEffort?: string | null;
	outcome?: string | null;
	autoMode?: boolean | null;
	surface?: string | null;
	durationMs?: number | null;
	totalTokens?: number | null;
	promptPreview: string;
	responsePreview: string;
	hasTranscript: boolean;
	createdAt: string;
}

export interface IPromptFeedbackFacetCount {
	key: string;
	count: number;
	negative: number;
}

export interface IPromptFeedbackTrendPoint {
	ts: string;
	positive: number;
	negative: number;
}

export interface IPromptFeedbackSummary {
	total: number;
	positive: number;
	negative: number;
	/** Share of ratings that were positive, 0..100. */
	satisfaction?: number | null;
	raters: number;
	conversations: number;
	withComment: number;
	byModel: IPromptFeedbackFacetCount[];
	byProvider: IPromptFeedbackFacetCount[];
	byOutcome: IPromptFeedbackFacetCount[];
	trend: IPromptFeedbackTrendPoint[];
}

export interface IPromptFeedbackFilters {
	providers: string[];
	models: string[];
	outcomes: string[];
}

export interface IPromptFeedbackResponse {
	items: IPromptFeedbackRecord[];
	total: number;
	page: number;
	pageSize: number;
	hours: number;
	/** True when the scan cap was reached and the summary covers only the most recent ratings. */
	truncated: boolean;
	summary: IPromptFeedbackSummary;
	filters: IPromptFeedbackFilters;
}

export interface IPromptFeedbackTranscriptEntry {
	role: string;
	content: string;
	timestamp: number;
}

export interface IPromptFeedbackDetail {
	record: IPromptFeedbackRecord;
	prompt: string;
	response: string;
	runContext?: Record<string, unknown> | null;
	usage?: Record<string, unknown> | null;
	steps: string[];
	tools: string[];
	appRefs: string[];
	transcript?: IPromptFeedbackTranscriptEntry[] | null;
	transcriptTruncated: boolean;
	canContact: boolean;
}

/**
 * Captured FlowScript applies that failed, were blocked, or applied with warnings.
 *
 * Mirrors `packages/api/src/routes/admin/telemetry/flowscript_failures.rs`. Unlike everything
 * above, these rows are user-attributed board content rather than anonymous counters — the source
 * they carry is stored redacted (declared values dropped, long literals generalized).
 */
export type IFlowScriptFailureOutcome = "error" | "blocked" | "partial";

export interface IFlowScriptFailureRecord {
	id: string;
	userId?: string | null;
	userName?: string | null;
	appId: string;
	boardId: string;
	layerId?: string | null;
	source: string;
	/** "editor" (a person in the FlowScript panel) or "agent" (FlowPilot). */
	origin: string;
	outcome: string;
	/** The one line that explains the row: the error, or the first diagnostic. */
	cause: string;
	errorMessage?: string | null;
	diagnostics: string[];
	diagnosticCount: number;
	commandCount: number;
	allowDeletions: boolean;
	flowscriptChars: number;
	droppedValues: number;
	redactedLiterals: number;
	truncated: boolean;
	appVersion?: string | null;
	platform?: string | null;
	traceId?: string | null;
	createdAt: string;
}

export interface IFlowScriptFailureFacet {
	key: string;
	/** Set for user facets, where the id alone is unreadable. */
	label?: string | null;
	count: number;
}

export interface IFlowScriptFailureSummary {
	total: number;
	errors: number;
	blocked: number;
	partial: number;
	users: number;
	apps: number;
	byOutcome: IFlowScriptFailureFacet[];
	bySource: IFlowScriptFailureFacet[];
	byOrigin: IFlowScriptFailureFacet[];
	byCause: IFlowScriptFailureFacet[];
	byUser: IFlowScriptFailureFacet[];
	byApp: IFlowScriptFailureFacet[];
}

export interface IFlowScriptFailureResponse {
	items: IFlowScriptFailureRecord[];
	total: number;
	page: number;
	pageSize: number;
	hours: number;
	summary: IFlowScriptFailureSummary;
}

export interface IFlowScriptFailureDetail {
	record: IFlowScriptFailureRecord;
	/** Redacted source, with line numbers matching what the user submitted. */
	flowscript: string;
	corrections: string[];
}
