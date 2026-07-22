import type { IBit } from "../schema";

/**
 * Provider names whose LLM/VLM models require a local llama.cpp runtime.
 * Hosts that cannot spawn llama.cpp (notably iOS) error when one of these is
 * selected, so they must be hidden from model pickers on such hosts.
 */
export const LOCAL_LLM_PROVIDER_NAMES: ReadonlySet<string> = new Set([
	"local",
	"llama.cpp",
	"llamacpp",
	"ollama",
]);

export function isLocalLlmModel(bit: IBit): boolean {
	const providerName = bit.parameters?.provider?.provider_name;
	return (
		typeof providerName === "string" &&
		LOCAL_LLM_PROVIDER_NAMES.has(providerName.toLowerCase())
	);
}

/** Return the normalized access tier declared by a hosted LLM/VLM bit. */
export function getLlmModelTier(bit: IBit): string | undefined {
	const tier = bit.parameters?.provider?.params?.tier;
	return typeof tier === "string" && tier.trim()
		? tier.trim().toUpperCase()
		: undefined;
}

export function isFreeLlmModel(bit: IBit): boolean {
	return getLlmModelTier(bit) === "FREE";
}

/**
 * Drop models that require a local llama.cpp runtime when the current host
 * cannot host one. On capable hosts the list is returned unchanged.
 */
export function filterHostableLlmModels(
	models: IBit[],
	canHostLlamaCPP: boolean,
): IBit[] {
	if (canHostLlamaCPP) return models;
	return models.filter((model) => !isLocalLlmModel(model));
}
