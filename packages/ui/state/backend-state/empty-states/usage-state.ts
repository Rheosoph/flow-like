import type {
	IEmbeddingUsageRecord,
	IExecutionUsageRecord,
	ILlmUsageRecord,
	IPaginatedResponse,
	IUsageSummary,
} from "@flow-like/flow-like-ui";
import type { IUsageState } from "../usage-state";

export class EmptyUsageState implements IUsageState {
	getLlmHistory(): Promise<IPaginatedResponse<ILlmUsageRecord>> {
		throw new Error("Method not implemented.");
	}
	getEmbeddingHistory(): Promise<IPaginatedResponse<IEmbeddingUsageRecord>> {
		throw new Error("Method not implemented.");
	}
	getExecutionHistory(): Promise<IPaginatedResponse<IExecutionUsageRecord>> {
		throw new Error("Method not implemented.");
	}
	getUsageSummary(): Promise<IUsageSummary> {
		throw new Error("Method not implemented.");
	}
}
