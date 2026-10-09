import { segment } from "./paths.js";
import type { HttpClient, SSEChunk } from "./client.js";
import type {
	AsyncInvokeResult,
	WorkflowTriggerOptions,
	InvokeBoardRequest,
} from "./types.js";

export function createWorkflowMethods(http: HttpClient) {
	return {
		triggerWorkflow(
			appId: string,
			boardId: string,
			nodeId: string,
			payload?: unknown,
			options?: WorkflowTriggerOptions,
		): AsyncIterable<SSEChunk> {
			const body: InvokeBoardRequest = {
				node_id: nodeId,
				payload,
				stream_state: options?.stream_state ?? true,
				version: options?.version,
				token: options?.token,
				oauth_tokens: options?.oauth_tokens,
				runtime_variables: options?.runtime_variables,
				profile_id: options?.profile_id,
			};
			return http.streamSSE(
				"POST",
				`/apps/${segment(appId)}/board/${segment(boardId)}/invoke`,
				{
					body,
					headers: options?.headers,
					signal: options?.signal,
					query: {
						local: options?.local ? "true" : undefined,
						isolated: options?.isolated ? "true" : undefined,
					},
				},
			);
		},

		async triggerWorkflowAsync(
			appId: string,
			boardId: string,
			nodeId: string,
			payload?: unknown,
			options?: WorkflowTriggerOptions,
		): Promise<AsyncInvokeResult> {
			const body: InvokeBoardRequest = {
				node_id: nodeId,
				payload,
				stream_state: options?.stream_state,
				version: options?.version,
				token: options?.token,
				oauth_tokens: options?.oauth_tokens,
				runtime_variables: options?.runtime_variables,
				profile_id: options?.profile_id,
			};
			return http.request<AsyncInvokeResult>(
				"POST",
				`/apps/${segment(appId)}/board/${segment(boardId)}/invoke/async`,
				{
					body,
					headers: options?.headers,
					signal: options?.signal,
				},
			);
		},
	};
}
