import type { IBit } from "../../lib/schema/bit/bit";

export function profileBitReference(bit: Pick<IBit, "hub" | "id">): string {
	return `${bit.hub}:${bit.id}`;
}

export function findProfileBit(reference: string, bits: readonly IBit[]) {
	return bits.find(
		(bit) =>
			profileBitReference(bit) === reference ||
			(!reference.includes(":") && bit.id === reference),
	);
}

export function appendProfileBitReference(
	values: readonly string[],
	reference: string,
): string[] {
	const trimmed = reference.trim();
	return trimmed && !values.includes(trimmed)
		? [...values, trimmed]
		: [...values];
}

export function profileBitDetails(bit: IBit) {
	const metadata = bit.meta?.en ?? Object.values(bit.meta ?? {})[0];
	const provider = bit.parameters?.provider;
	return {
		name: metadata?.name || bit.model_slug || bit.id,
		description: metadata?.description || "",
		provider:
			typeof provider?.provider_name === "string"
				? provider.provider_name
				: typeof provider === "string"
					? provider
					: bit.model_evaluation?.creator_name || bit.hub,
	};
}

export function profileBitTypeLabel(type: string): string {
	const labels: Record<string, string> = {
		Llm: "Language models",
		Vlm: "Vision models",
		Stt: "Speech to text",
		Tts: "Text to speech",
		Embedding: "Text embeddings",
		ImageEmbedding: "Image embeddings",
		ImageGeneration: "Image generation",
		VideoGeneration: "Video generation",
	};
	return labels[type] ?? type.replace(/([a-z])([A-Z])/g, "$1 $2");
}
