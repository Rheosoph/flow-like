import type { HttpClient, SSEChunk } from "./client.js";
import type {
	ChatCompletionOptions,
	ChatCompletionResult,
	ChatMessage,
	ChatUsage,
	ResponsesOptions,
	ResponsesResult,
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

		/**
		 * Call a Bit whose provider declares the `Responses` API surface.
		 *
		 * `input` follows the OpenAI Responses schema — a plain string or a list
		 * of input items. Bits that speak Chat Completions are rejected by the
		 * server; use `chatCompletions` for those.
		 */
		async responses(
			input: unknown,
			bitId: string,
			options?: ResponsesOptions,
		): Promise<ResponsesResult> {
			return http.request<ResponsesResult>("POST", "/responses", {
				body: {
					input,
					model: bitId,
					instructions: options?.instructions,
					temperature: options?.temperature,
					max_output_tokens: options?.max_output_tokens,
					top_p: options?.top_p,
					tools: options?.tools,
					stream: false,
				},
				signal: options?.signal,
			});
		},

		responsesStream(
			input: unknown,
			bitId: string,
			options?: ResponsesOptions,
		): AsyncIterable<SSEChunk> {
			return http.streamSSE("POST", "/responses", {
				body: {
					input,
					model: bitId,
					instructions: options?.instructions,
					temperature: options?.temperature,
					max_output_tokens: options?.max_output_tokens,
					top_p: options?.top_p,
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
