import { isHostedLlmProviderName } from "../../../../lib/bit/local-model-filter";
import type { IBit } from "../../../../lib/schema/bit/bit";

export type ModelAccess =
	| "local"
	| "local_with_hosted_fallback"
	| "hosted"
	| "unknown";

const object = (value: unknown): Record<string, unknown> | undefined =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;

const present = (value: unknown): boolean =>
	typeof value === "string" && value.trim().length > 0;

/** Model metadata describes available routes; the target's runtime decides whether it can run locally. */
export function modelAccess(bit: IBit): ModelAccess {
	const parameters = object(bit.parameters);
	const provider = object(parameters?.provider);
	const name = provider?.provider_name;
	if (typeof name !== "string" || !name.trim()) return "unknown";
	const normalized = name.trim().toLowerCase();
	const local = normalized === "local";
	const hosted = isHostedLlmProviderName(normalized);
	if (bit.type === "Llm" || bit.type === "Vlm") {
		if (local || normalized === "mlx") return "local";
		return hosted ? "hosted" : "unknown";
	}
	// The image embedding proxy accepts text inputs only, so remote metadata cannot enable image calls.
	if (bit.type === "ImageEmbedding") return local ? "local" : "unknown";
	if (bit.type !== "Embedding") return "unknown";
	const remote = object(parameters?.remote);
	if (parameters?.remote != null && !remote) return "unknown";
	const remoteModel = present(remote?.model_id) || present(provider?.model_id);
	const supportsHosted = (!!remote || hosted) && remoteModel;
	if (local) return supportsHosted ? "local_with_hosted_fallback" : "local";
	return supportsHosted ? "hosted" : "unknown";
}
