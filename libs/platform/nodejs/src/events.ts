import type { HttpClient, SSEChunk } from "./client.js";
import type { AsyncInvokeResult, TriggerOptions } from "./types.js";

export function createEventMethods(http: HttpClient) {
	return {
		triggerEvent(
			appId: string,
			eventId: string,
			payload?: unknown,
			options?: TriggerOptions,
		): AsyncIterable<SSEChunk> {
			return http.streamSSE("POST", `/apps/${appId}/events/${eventId}/invoke`, {
				body: payload,
				headers: options?.headers,
				signal: options?.signal,
			});
		},

		async triggerEventAsync(
			appId: string,
			eventId: string,
			payload?: unknown,
			options?: TriggerOptions,
		): Promise<AsyncInvokeResult> {
			return http.request<AsyncInvokeResult>(
				"POST",
				`/apps/${appId}/events/${eventId}/invoke-async`,
				{
					body: payload,
					headers: options?.headers,
					signal: options?.signal,
				},
			);
		},
	};
}
