import type {
	IEmbeddingUsageRecord,
	IExecutionUsageRecord,
	ILlmUsageRecord,
	IPaginatedResponse,
	IUsageState,
	IUsageSummary,
} from "@flow-like/flow-like-ui";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class UsageState implements IUsageState {
	constructor(private readonly backend: TauriBackend) {}

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

		return fetcher<IPaginatedResponse<ILlmUsageRecord>>(
			this.backend.profile!,
			`usage/llm?${params}`,
			{ method: "GET" },
			this.backend.auth,
		);
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

		return fetcher<IPaginatedResponse<IEmbeddingUsageRecord>>(
			this.backend.profile!,
			`usage/embeddings?${params}`,
			{ method: "GET" },
			this.backend.auth,
		);
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

		return fetcher<IPaginatedResponse<IExecutionUsageRecord>>(
			this.backend.profile!,
			`usage/executions?${params}`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async getUsageSummary(): Promise<IUsageSummary> {
		return fetcher<IUsageSummary>(
			this.backend.profile!,
			"usage/summary",
			{ method: "GET" },
			this.backend.auth,
		);
	}
}
