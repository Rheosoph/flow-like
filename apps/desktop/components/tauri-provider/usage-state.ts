import type {
	IEmbeddingUsageRecord,
	IExecutionActivity,
	IExecutionUsageRecord,
	ILlmUsageRecord,
	IPaginatedResponse,
	IUsageState,
	IUsageSummary,
} from "@flow-like/flow-like-ui";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function expectPage<T>(
	page: IPaginatedResponse<T>,
	route: string,
): IPaginatedResponse<T> {
	if (!isRecord(page)) throw new Error(`Unexpected response from ${route}`);
	return { ...page, items: asArray(page.items) };
}

export class UsageState implements IUsageState {
	constructor(private readonly backend: TauriBackend) {}

	private profile(): NonNullable<TauriBackend["profile"]> {
		const profile = this.backend.profile;
		if (!profile) throw new Error("Profile context is not available");
		return profile;
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

		return expectPage(
			await fetcher<IPaginatedResponse<ILlmUsageRecord>>(
				this.profile(),
				`usage/llm?${params}`,
				{ method: "GET" },
				this.backend.auth,
			),
			"usage/llm",
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

		return expectPage(
			await fetcher<IPaginatedResponse<IEmbeddingUsageRecord>>(
				this.profile(),
				`usage/embeddings?${params}`,
				{ method: "GET" },
				this.backend.auth,
			),
			"usage/embeddings",
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

		return expectPage(
			await fetcher<IPaginatedResponse<IExecutionUsageRecord>>(
				this.profile(),
				`usage/executions?${params}`,
				{ method: "GET" },
				this.backend.auth,
			),
			"usage/executions",
		);
	}

	async getExecutionActivity(
		days = 7,
		appId?: string,
	): Promise<IExecutionActivity> {
		const params = new URLSearchParams({ days: String(days) });
		if (appId) params.set("app_id", appId);

		const activity = await fetcher<IExecutionActivity>(
			this.profile(),
			`usage/executions/activity?${params}`,
			{ method: "GET" },
			this.backend.auth,
		);
		if (!isRecord(activity)) {
			throw new Error("Unexpected response from usage/executions/activity");
		}
		return {
			...activity,
			buckets: asArray(activity.buckets),
			apps: asArray(activity.apps),
			attention: asArray(activity.attention),
		};
	}

	async getUsageSummary(): Promise<IUsageSummary> {
		const summary = await fetcher<IUsageSummary>(
			this.profile(),
			"usage/summary",
			{ method: "GET" },
			this.backend.auth,
		);
		if (!isRecord(summary)) {
			throw new Error("Unexpected response from usage/summary");
		}
		return summary;
	}
}
