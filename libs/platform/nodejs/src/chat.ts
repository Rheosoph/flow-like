import type { HttpClient, SSEChunk } from "./client.js";
import type {
	ChatCompletionOptions,
	ChatCompletionResult,
	ChatMessage,
	ChatUsage,
} from "./types.js";

export function createChatMethods(http: HttpClient) {
	return {
		async chatCompletions(
			messages: ChatMessage[],
			bitId: string,
			options?: ChatCompletionOptions,
		): Promise<ChatCompletionResult> {
			return http.request<ChatCompletionResult>("POST", "/chat/completions", {
				body: {
					messages,
					model: bitId,
					temperature: options?.temperature,
					max_tokens: options?.max_tokens,
					top_p: options?.top_p,
					stop: options?.stop,
					tools: options?.tools,
					stream: false,
				},
				signal: options?.signal,
			});
		},

		chatCompletionsStream(
			messages: ChatMessage[],
			bitId: string,
			options?: ChatCompletionOptions,
		): AsyncIterable<SSEChunk> {
			return http.streamSSE("POST", "/chat/completions", {
				body: {
					messages,
					model: bitId,
					temperature: options?.temperature,
					max_tokens: options?.max_tokens,
					top_p: options?.top_p,
					stop: options?.stop,
					tools: options?.tools,
					stream: true,
				},
				signal: options?.signal,
			});
		},

		async getUsage(): Promise<ChatUsage> {
			return http.request<ChatUsage>("GET", "/chat/usage");
		},
	};
}
