import type {
	IEmbeddingUsageRecord,
	IExecutionActivity,
	IExecutionUsageRecord,
	ILlmUsageRecord,
	IPaginatedResponse,
	IRecentExecutionsResponse,
	IUsageRequestOptions,
	IUsageState,
	IUsageSummary,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, apiGet } from "./api-utils";

export class WebUsageState implements IUsageState {
	constructor(private readonly backend: WebBackendRef) {}

	private get<T>(path: string, options?: IUsageRequestOptions): Promise<T> {
		return options?.fresh
			? apiGet(path, this.backend.auth, {
					headers: { "Cache-Control": "no-cache" },
				})
			: apiGet(path, this.backend.auth);
	}

	async getLlmHistory(
		page = 0,
		pageSize = 50,
		appId?: string,
	): Promise<IPaginatedResponse<ILlmUsageRecord>> {
		const params = new URLSearchParams({
			page: String(page),
			page_size: String(pageSize),
		});
		if (appId) params.set("app_id", appId);
		return apiGet(`usage/llm?${params}`, this.backend.auth);
	}

	async getEmbeddingHistory(
		page = 0,
		pageSize = 50,
		appId?: string,
	): Promise<IPaginatedResponse<IEmbeddingUsageRecord>> {
		const params = new URLSearchParams({
			page: String(page),
			page_size: String(pageSize),
		});
		if (appId) params.set("app_id", appId);
		return apiGet(`usage/embeddings?${params}`, this.backend.auth);
	}

	async getExecutionHistory(
		page = 0,
		pageSize = 50,
		appId?: string,
		options?: IUsageRequestOptions,
	): Promise<IPaginatedResponse<IExecutionUsageRecord>> {
		const params = new URLSearchParams({
			page: String(page),
			page_size: String(pageSize),
		});
		if (appId) params.set("app_id", appId);
		return this.get(`usage/executions?${params}`, options);
	}

	async getRecentExecutions(
		limit = 50,
		appId?: string,
		options?: IUsageRequestOptions,
	): Promise<IRecentExecutionsResponse> {
		const params = new URLSearchParams({ limit: String(limit) });
		if (appId) params.set("app_id", appId);
		return this.get(`usage/executions/recent?${params}`, options);
	}

	async getExecutionActivity(
		days = 7,
		appId?: string,
		options?: IUsageRequestOptions,
	): Promise<IExecutionActivity> {
		const params = new URLSearchParams({ days: String(days) });
		if (appId) params.set("app_id", appId);
		return this.get(`usage/executions/activity?${params}`, options);
	}

	async getUsageSummary(
		options?: IUsageRequestOptions,
	): Promise<IUsageSummary> {
		return this.get("usage/summary", options);
	}
}
