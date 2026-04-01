import type { HttpClient } from "./client.js";
import type { Bit, BitSearchQuery, ModelInfo } from "./types.js";

function extractModelInfo(bit: Bit, lang = "en"): ModelInfo {
	const meta = bit.meta?.[lang] ?? Object.values(bit.meta ?? {})[0];
	const params = bit.parameters as Record<string, unknown> | undefined;
	const provider = (params?.provider ?? params) as
		| Record<string, unknown>
		| undefined;

	return {
		bit_id: bit.id,
		name: meta?.name ?? bit.id,
		description: meta?.description ?? "",
		provider_name: provider?.provider_name as string | undefined,
		model_id: provider?.model_id as string | undefined,
		context_length: params?.context_length as number | undefined,
		vector_length: params?.vector_length as number | undefined,
		languages: params?.languages as string[] | undefined,
		tags: meta?.tags ?? [],
	};
}

export function createBitMethods(http: HttpClient) {
	return {
		async searchBits(query: BitSearchQuery): Promise<Bit[]> {
			return http.request<Bit[]>("POST", "/bit", { body: query });
		},

		async getBit(bitId: string): Promise<Bit> {
			return http.request<Bit>("GET", `/bit/${bitId}`);
		},

		async listLlms(search?: string, limit = 50): Promise<ModelInfo[]> {
			const bits = await http.request<Bit[]>("POST", "/bit", {
				body: {
					bit_types: ["Llm", "Vlm"],
					search,
					limit,
				} satisfies BitSearchQuery,
			});
			return bits
				.filter(
					(b) => (b.type === "Llm" || b.type === "Vlm") && hasRemoteProvider(b),
				)
				.map((b) => extractModelInfo(b));
		},

		async listEmbeddingModels(
			search?: string,
			limit = 50,
		): Promise<ModelInfo[]> {
			const bits = await http.request<Bit[]>("POST", "/bit", {
				body: {
					bit_types: ["Embedding"],
					search,
					limit,
				} satisfies BitSearchQuery,
			});
			return bits
				.filter((b) => b.type === "Embedding" && hasRemoteProvider(b))
				.map((b) => extractModelInfo(b));
		},
	};
}

function hasRemoteProvider(bit: Bit): boolean {
	const params = bit.parameters as Record<string, unknown> | undefined;
	if (!params) return false;

	// LLM/VLM pattern: provider_name starts with "hosted"
	const provider =
		(params.provider as Record<string, unknown> | undefined) ?? params;
	const name = provider?.provider_name as string | undefined;
	if (name && name.toLowerCase().startsWith("hosted")) return true;

	// Embedding pattern: remote config with endpoint + implementation
	const remote = params.remote as Record<string, unknown> | undefined;
	if (remote?.endpoint && remote?.implementation) return true;

	return false;
}
