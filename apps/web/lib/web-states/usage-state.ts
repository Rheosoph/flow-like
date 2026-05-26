import type {
	IEmbeddingUsageRecord,
	IExecutionUsageRecord,
	ILlmUsageRecord,
	IPaginatedResponse,
	IUsageState,
	IUsageSummary,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, apiGet } from "./api-utils";

export class WebUsageState implements IUsageState {
	constructor(private readonly backend: WebBackendRef) {}

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
	): Promise<IPaginatedResponse<IExecutionUsageRecord>> {
		const params = new URLSearchParams({
			page: String(page),
			page_size: String(pageSize),
		});
		if (appId) params.set("app_id", appId);
		return apiGet(`usage/executions?${params}`, this.backend.auth);
	}

	async getUsageSummary(): Promise<IUsageSummary> {
		return apiGet("usage/summary", this.backend.auth);
	}
}
