import type { HttpClient, SSEChunk } from "./client.js";
import type {
	AsyncInvokeResult,
	InvokeBoardQuery,
	InvokeBoardRequest,
	TriggerOptions,
} from "./types.js";

export function createWorkflowMethods(http: HttpClient) {
	return {
		triggerWorkflow(
			appId: string,
			boardId: string,
			nodeId: string,
			payload?: unknown,
			options?: TriggerOptions & InvokeBoardQuery,
		): AsyncIterable<SSEChunk> {
			const body: InvokeBoardRequest = {
				node_id: nodeId,
				payload,
				stream_state: true,
			};
			return http.streamSSE("POST", `/apps/${appId}/board/${boardId}/invoke`, {
				body,
				headers: options?.headers,
				signal: options?.signal,
				query: {
					local: options?.local ? "true" : undefined,
					isolated: options?.isolated ? "true" : undefined,
				},
			});
		},

		async triggerWorkflowAsync(
			appId: string,
			boardId: string,
			nodeId: string,
			payload?: unknown,
			options?: TriggerOptions,
		): Promise<AsyncInvokeResult> {
			const body: InvokeBoardRequest = {
				node_id: nodeId,
				payload,
			};
			return http.request<AsyncInvokeResult>(
				"POST",
				`/apps/${appId}/board/${boardId}/invoke/async`,
				{
					body,
					headers: options?.headers,
					signal: options?.signal,
				},
			);
		},
	};
}
