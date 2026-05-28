export interface ILlmUsageRecord {
	id: string;
	model_id: string;
	provider: string | null;
	endpoint: string | null;
	token_in: number;
	token_out: number;
	latency: number | null;
	app_id: string | null;
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
	created_at: string;
}

export interface IPaginatedResponse<T> {
	items: T[];
	total: number;
	page: number;
	page_size: number;
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
	limits: IAppUsageLimits | null;
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
	apps: IAdminAppUsage[];
	models: IAdminModelUsage[];
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
