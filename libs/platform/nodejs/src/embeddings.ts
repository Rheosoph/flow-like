import type { HttpClient } from "./client.js";
import type { EmbedOptions, EmbedResult } from "./types.js";

export function createEmbeddingMethods(http: HttpClient) {
	return {
		async embed(
			bitId: string,
			input: string | string[],
			options?: EmbedOptions,
		): Promise<EmbedResult> {
			const texts = Array.isArray(input) ? input : [input];
			return http.request<EmbedResult>("POST", "/embeddings/embed", {
				body: {
					model: bitId,
					input: texts,
					embed_type: options?.embed_type ?? "query",
				},
				signal: options?.signal,
			});
		},
	};
}
