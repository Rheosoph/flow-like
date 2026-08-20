import type { IBit } from "../schema";
import { IBitTypes } from "../schema";

/** Provider names whose LLM/VLM models require a local llama.cpp runtime. */
export const LLAMA_CPP_PROVIDER_NAMES: ReadonlySet<string> = new Set([
	"local",
	"llama.cpp",
	"llamacpp",
	"ollama",
]);

/** Provider names whose LLM/VLM models require Apple's MLX runtime. */
export const MLX_PROVIDER_NAMES: ReadonlySet<string> = new Set(["mlx"]);

/** @deprecated Prefer the runtime-specific provider sets. */
export const LOCAL_LLM_PROVIDER_NAMES: ReadonlySet<string> =
	LLAMA_CPP_PROVIDER_NAMES;

export interface LocalModelHostCapabilities {
	canHostLlamaCPP: boolean;
	canHostMLX: boolean;
}

function normalizedProviderName(bit: IBit): string | undefined {
	const providerName = bit.parameters?.provider?.provider_name;
	return typeof providerName === "string"
		? providerName.trim().toLowerCase()
		: undefined;
}

export function isLlamaCppLlmModel(bit: IBit): boolean {
	const providerName = normalizedProviderName(bit);
	return (
		typeof providerName === "string" &&
		LLAMA_CPP_PROVIDER_NAMES.has(providerName)
	);
}

export function isMlxLlmModel(bit: IBit): boolean {
	const providerName = normalizedProviderName(bit);
	return (
		typeof providerName === "string" && MLX_PROVIDER_NAMES.has(providerName)
	);
}

export function isLocalLlmModel(bit: IBit): boolean {
	return isLlamaCppLlmModel(bit) || isMlxLlmModel(bit);
}

export function isHostableLlmModel(
	bit: IBit,
	capabilities: LocalModelHostCapabilities,
): boolean {
	if (isMlxLlmModel(bit)) return capabilities.canHostMLX;
	if (isLlamaCppLlmModel(bit)) return capabilities.canHostLlamaCPP;
	return true;
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
 * Drop local models whose runtime is unavailable on the current host.
 */
export function filterHostableLlmModels(
	models: IBit[],
	capabilities: LocalModelHostCapabilities,
): IBit[] {
	if (capabilities.canHostLlamaCPP && capabilities.canHostMLX) return models;
	return models.filter((model) => isHostableLlmModel(model, capabilities));
}

/**
 * Profile bit references are stored as `<hub>:<id>`, but the hub a bit *reports*
 * is not necessarily the hub it is served from — a bit reachable through
 * `api.flow-like.com` can still carry `api.alpha.flow-like.com` in `hub`. Every
 * other surface (model cards, the model catalog, the hub's own
 * `/profile/{id}/bits`) therefore decides membership on the id alone.
 */
export function profileBitIds(
	refs: readonly string[] | undefined,
): Set<string> {
	return new Set((refs ?? []).map((ref) => ref.split(":").pop() ?? ref));
}

const LLM_BIT_TYPES: ReadonlySet<string> = new Set([
	IBitTypes.Llm,
	IBitTypes.Vlm,
]);

/**
 * The LLM/VLM models a FlowPilot surface may offer: the profile's own models
 * plus the user's custom library, minus runtimes this host cannot serve.
 */
export function selectProfileLlmModels(
	catalogBits: readonly IBit[] | undefined,
	customBits: readonly IBit[] | undefined,
	profileBitRefs: readonly string[] | undefined,
	capabilities: LocalModelHostCapabilities,
): IBit[] {
	if (!catalogBits || !profileBitRefs) return [];
	const ids = profileBitIds(profileBitRefs);
	const profileModels = catalogBits.filter(
		(bit) => ids.has(bit.id) && LLM_BIT_TYPES.has(bit.type),
	);
	const seen = new Set(profileModels.map((bit) => bit.id));
	const ownModels = (customBits ?? []).filter(
		(bit) => !seen.has(bit.id) && LLM_BIT_TYPES.has(bit.type),
	);
	return filterHostableLlmModels(
		[...ownModels, ...profileModels],
		capabilities,
	);
}
