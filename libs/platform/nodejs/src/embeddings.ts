import type { HttpClient } from "./client.js";
import type { EmbedOptions, EmbedResult, EmbeddingInput } from "./types.js";

export function createEmbeddingMethods(http: HttpClient) {
	return {
		async embed(
			bitId: string,
			input: EmbeddingInput | EmbeddingInput[],
			options?: EmbedOptions,
		): Promise<EmbedResult> {
			const inputs = Array.isArray(input) ? input : [input];
			return http.request<EmbedResult>("POST", "/embeddings/embed", {
				body: {
					model: bitId,
					input: inputs,
					embed_type: options?.embed_type ?? "query",
				},
				signal: options?.signal,
			});
		},
	};
}
