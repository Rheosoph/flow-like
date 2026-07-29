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
