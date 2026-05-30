export interface IAnalyticsOverview {
	totalExecutions: number;
	successfulExecutions: number;
	failedExecutions: number;
	uniqueUsers: number;
	avgFeedbackRating: number | null;
	totalFeedback: number;
	positiveFeedback: number;
	negativeFeedback: number;
	totalLlmCost: number;
	totalEmbeddingCost: number;
	avgLatencyMs: number | null;
	periodExecutions: number;
	periodUniqueUsers: number;
	executionsChangePercent: number | null;
	usersChangePercent: number | null;
}

export interface IDailyAnalyticsStat {
	date: string;
	executions: number;
	successfulExecutions: number;
	failedExecutions: number;
	uniqueUsers: number;
	feedbackCount: number;
	avgRating: number | null;
	llmCost: number;
	embeddingCost: number;
	avgLatency: number | null;
	p95Latency: number | null;
	positiveFeedback: number;
	negativeFeedback: number;
}

export interface IAnalyticsStats {
	dailyStats: IDailyAnalyticsStat[];
	summary: IAnalyticsOverview;
}

export interface IAnalyticsDashboard {
	overview: IAnalyticsOverview;
	stats: IAnalyticsStats;
}

export interface IFeedbackItem {
	id: string;
	userId: string | null;
	eventId: string | null;
	rating: number;
	comment: string;
	createdAt: string;
}

export interface IPaginatedFeedback {
	items: IFeedbackItem[];
	total: number;
	offset: number;
	limit: number;
}

export interface IAnalyticsState {
	getAnalyticsOverview(appId: string): Promise<IAnalyticsOverview>;

	getAnalyticsDashboard(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
	): Promise<IAnalyticsDashboard>;

	getAnalyticsStats(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
	): Promise<IAnalyticsStats>;

	listFeedback(
		appId: string,
		offset?: number,
		limit?: number,
		minRating?: number,
		maxRating?: number,
	): Promise<IPaginatedFeedback>;
}
