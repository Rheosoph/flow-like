export interface ILlmUsageRecord {
	id: string;
	model_id: string;
	provider: string | null;
	endpoint: string | null;
	token_in: number;
	token_out: number;
	latency: number | null;
	app_id: string | null;
	technical_user_id: string | null;
	price: number;
	created_at: string;
}

export interface IEmbeddingUsageRecord {
	id: string;
	model_id: string;
	provider: string | null;
	endpoint: string | null;
	token_count: number;
	latency: number | null;
	app_id: string | null;
	technical_user_id: string | null;
	price: number;
	created_at: string;
}

export interface IExecutionUsageRecord {
	id: string;
	instance: string | null;
	board_id: string;
	node_id: string;
	version: string;
	microseconds: number;
	status: string;
	app_id: string | null;
	technical_user_id: string | null;
	created_at: string;
}

export interface IRecentExecutionsResponse {
	items: IExecutionUsageRecord[];
}

export interface IPaginatedResponse<T> {
	items: T[];
	total: number;
	page: number;
	page_size: number;
}

export interface IExecutionActivityBucket {
	day: string;
	count: number;
	attention_count: number;
}

export interface IExecutionActivityApp {
	app_id: string | null;
	count: number;
	attention_count: number;
}

/**
 * Counts for a whole period, aggregated server-side. Unlike a page of
 * {@link IExecutionUsageRecord}, every number here describes the window rather
 * than the page size, so `total` is a real count and an empty `buckets` entry
 * means a quiet day rather than a day the page never reached.
 */
export interface IExecutionActivity {
	days: number;
	from: string;
	to: string;
	buckets: IExecutionActivityBucket[];
	apps: IExecutionActivityApp[];
	total: number;
	attention_total: number;
	average_microseconds: number | null;
	/** Newest flagged records in the window, capped for display. */
	attention: IExecutionUsageRecord[];
}

export interface IUsageSummary {
	total_llm_price: number;
	total_embedding_price: number;
	total_llm_invocations: number;
	total_embedding_invocations: number;
	total_executions: number;
}

export type IUsageLimitPeriod = "weekly" | "monthly" | "yearly";

export interface IAppUsageLimitWindow {
	costMicroDollars: number | null;
	tokenLimit: number | null;
	enabled: boolean;
	hard: boolean;
	warningThresholdPercent: number | null;
}

export interface IAppUsageLimits {
	weekly: IAppUsageLimitWindow;
	monthly: IAppUsageLimitWindow;
	yearly: IAppUsageLimitWindow;
}

/** One serverless function billed for part of an execution. */
export interface IComputeLeg {
	architecture: string;
	memoryGb: number;
	durationShare: number;
	microDollarsPerGbSecond: number;
}

/** Rate card the backend used to estimate runtime cost. */
export interface IComputeCostModel {
	legs: IComputeLeg[];
	requestMicroDollars: number;
	multiplier: number;
	microDollarsPerSecond: number;
	microDollarsPerExecution: number;
}

export interface IAdminUsageTotals {
	llmPrice: number;
	embeddingPrice: number;
	totalPrice: number;
	llmTokens: number;
	embeddingTokens: number;
	totalTokens: number;
	llmInvocations: number;
	embeddingInvocations: number;
	executions: number;
	executionMicroseconds: number;
	averageExecutionMs: number | null;
	computeCost: number;
}

export interface IAdminUserUsage {
	userId: string | null;
	displayName: string | null;
	email: string | null;
	llmPrice: number;
	embeddingPrice: number;
	totalPrice: number;
	llmTokens: number;
	embeddingTokens: number;
	totalTokens: number;
	llmInvocations: number;
	embeddingInvocations: number;
	executions: number;
	executionMicroseconds: number;
	averageExecutionMs: number | null;
	computeCost: number;
}

export interface IAdminAppUsage {
	appId: string | null;
	appName: string | null;
	llmPrice: number;
	embeddingPrice: number;
	totalPrice: number;
	llmTokens: number;
	embeddingTokens: number;
	totalTokens: number;
	llmInvocations: number;
	embeddingInvocations: number;
	executions: number;
	executionMicroseconds: number;
	averageExecutionMs: number | null;
	computeCost: number;
	limits: IAppUsageLimits | null;
}

export interface IAdminTechnicalUserUsage {
	technicalUserId: string;
	name: string | null;
	appId: string | null;
	appName: string | null;
	creatorUserId: string | null;
	creatorMembershipId: string | null;
	creatorDisplayName: string | null;
	creatorEmail: string | null;
	limits: IAppUsageLimits | null;
	llmPrice: number;
	embeddingPrice: number;
	totalPrice: number;
	llmTokens: number;
	embeddingTokens: number;
	totalTokens: number;
	llmInvocations: number;
	embeddingInvocations: number;
	executions: number;
	executionMicroseconds: number;
	averageExecutionMs: number | null;
	computeCost: number;
}

export interface IAdminModelUsage {
	kind: "llm" | "embedding";
	modelId: string;
	provider: string | null;
	endpoint: string | null;
	price: number;
	tokens: number;
	invocations: number;
	averageLatencyMs: number | null;
}

export interface IAdminUserStats {
	totalUsers: number;
	newUsersToday: number;
	newUsersWeekly: number;
	newUsersMonthly: number;
	activeUsersDaily: number;
	activeUsersWeekly: number;
	activeUsersMonthly: number;
	activeAppsDaily: number;
	activeAppsWeekly: number;
	activeAppsMonthly: number;
	aiUsersMonthly: number;
	executionUsersMonthly: number;
	powerUsersWeekly: number;
	powerUsersMonthly: number;
	averageCostPerActiveUser: number | null;
}

export interface IAdminUsageTrendPoint {
	bucket: string;
	label: string;
	newUsers: number;
	activeUsers: number;
	executions: number;
	aiInvocations: number;
	tokens: number;
	cost: number;
}

export interface IAdminPowerUser {
	userId: string;
	displayName: string | null;
	email: string | null;
	totalPrice: number;
	totalTokens: number;
	aiInvocations: number;
	executions: number;
	totalInteractions: number;
	activeDays: number;
	lastSeen: string | null;
}

export interface IAdminUsageOverview {
	period: IUsageLimitPeriod;
	startedAt: string;
	totals: IAdminUsageTotals;
	userStats: IAdminUserStats;
	trend: IAdminUsageTrendPoint[];
	powerUsers: IAdminPowerUser[];
	users: IAdminUserUsage[];
	technicalUsers: IAdminTechnicalUserUsage[];
	apps: IAdminAppUsage[];
	models: IAdminModelUsage[];
	computeCostModel: IComputeCostModel;
}

export interface IAdminPaginated<T> {
	items: T[];
	total: number;
	page: number;
	pageSize: number;
}

export interface IAdminUsageInvocation {
	id: string;
	kind: string;
	status: string;
	userId: string | null;
	technicalUserId: string | null;
	appId: string | null;
	provider: string | null;
	endpoint: string | null;
	modelId: string | null;
	providerRequestId: string | null;
	estimatedTokens: number;
	estimatedCostMicroDollars: number;
	inputTokens: number;
	outputTokens: number;
	embeddingTokens: number;
	costMicroDollars: number;
	latency: number | null;
	error: string | null;
	startedAt: string;
	completedAt: string | null;
}

export interface IAdminUsageAlert {
	id: string;
	kind: string;
	severity: string;
	period: string | null;
	message: string;
	appId: string | null;
	userId: string | null;
	thresholdPercent: number | null;
	currentCostMicroDollars: number | null;
	currentTokens: number | null;
	acknowledgedAt: string | null;
	createdAt: string;
}

export interface IAdminUsageAuditLog {
	id: string;
	appId: string | null;
	userId: string | null;
	actorUserId: string | null;
	action: string;
	before: unknown | null;
	after: unknown | null;
	createdAt: string;
}

export interface IUsageReconciliationResult {
	olderThanMinutes: number;
	markedUnknownUsage: number;
}
