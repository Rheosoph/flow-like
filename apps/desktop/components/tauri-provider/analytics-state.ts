import type {
	IAnalyticsDashboard,
	IAnalyticsOverview,
	IAnalyticsState,
	IAnalyticsStats,
	IPaginatedFeedback,
} from "@flow-like/flow-like-ui/state/backend-state/analytics-state";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class AnalyticsState implements IAnalyticsState {
	constructor(private readonly backend: TauriBackend) {}

	async getAnalyticsOverview(
		appId: string,
		eventId?: string,
	): Promise<IAnalyticsOverview> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const params = new URLSearchParams();
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/?${query}`
			: `apps/${appId}/analytics/`;
		return await fetcher<IAnalyticsOverview>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
	}

	async getAnalyticsDashboard(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
		eventId?: string,
	): Promise<IAnalyticsDashboard> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const params = new URLSearchParams();
		if (startDate) params.set("start_date", startDate);
		if (endDate) params.set("end_date", endDate);
		if (period) params.set("period", period);
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/dashboard?${query}`
			: `apps/${appId}/analytics/dashboard`;
		return await fetcher<IAnalyticsDashboard>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
	}

	async getAnalyticsStats(
		appId: string,
		startDate?: string,
		endDate?: string,
		period?: "day" | "week" | "month",
		eventId?: string,
	): Promise<IAnalyticsStats> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
		const params = new URLSearchParams();
		if (startDate) params.set("start_date", startDate);
		if (endDate) params.set("end_date", endDate);
		if (period) params.set("period", period);
		if (eventId) params.set("event_id", eventId);

		const query = params.toString();
		const url = query
			? `apps/${appId}/analytics/stats?${query}`
			: `apps/${appId}/analytics/stats`;
		return await fetcher<IAnalyticsStats>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
	}

	async listFeedback(
		appId: string,
		offset?: number,
		limit?: number,
		minRating?: number,
		maxRating?: number,
		eventId?: string,
	): Promise<IPaginatedFeedback> {
		if (!this.backend.profile) {
			throw new Error("Profile not available");
		}
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
		return await fetcher<IPaginatedFeedback>(
			this.backend.profile,
			url,
			undefined,
			this.backend.auth,
		);
	}
}
