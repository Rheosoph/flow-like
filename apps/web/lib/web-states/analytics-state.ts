import type {
	IAnalyticsDashboard,
	IAnalyticsOverview,
	IAnalyticsState,
	IAnalyticsStats,
	IPaginatedFeedback,
} from "@flow-like/flow-like-ui/state/backend-state/analytics-state";
import { type WebBackendRef, apiGet } from "./api-utils";

export class WebAnalyticsState implements IAnalyticsState {
	constructor(private readonly backend: WebBackendRef) {}

	async getAnalyticsOverview(
		appId: string,
		eventId?: string,
	): Promise<IAnalyticsOverview> {
		const params = new URLSearchParams();
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/?${query}`
			: `apps/${appId}/analytics/`;
		return await apiGet<IAnalyticsOverview>(url, this.backend.auth);
	}

	async getAnalyticsDashboard(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
		eventId?: string,
	): Promise<IAnalyticsDashboard> {
		const params = new URLSearchParams();
		if (startDate) params.set("start_date", startDate);
		if (endDate) params.set("end_date", endDate);
		if (period) params.set("period", period);
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/dashboard?${query}`
			: `apps/${appId}/analytics/dashboard`;
		return await apiGet<IAnalyticsDashboard>(url, this.backend.auth);
	}

	async getAnalyticsStats(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
		eventId?: string,
	): Promise<IAnalyticsStats> {
		const params = new URLSearchParams();
		if (startDate) params.set("start_date", startDate);
		if (endDate) params.set("end_date", endDate);
		if (period) params.set("period", period);
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/stats?${query}`
			: `apps/${appId}/analytics/stats`;
		return await apiGet<IAnalyticsStats>(url, this.backend.auth);
	}

	async listFeedback(
		appId: string,
		offset?: number,
		limit?: number,
		minRating?: number,
		maxRating?: number,
		eventId?: string,
	): Promise<IPaginatedFeedback> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());
		if (minRating !== undefined) params.set("min_rating", minRating.toString());
		if (maxRating !== undefined) params.set("max_rating", maxRating.toString());
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/feedback?${query}`
			: `apps/${appId}/analytics/feedback`;
		return await apiGet<IPaginatedFeedback>(url, this.backend.auth);
	}
}
