import { type IBit, IBitTypes } from "../schema/bit/bit";

export const MODEL_QUALITY_CATEGORIES = {
	[IBitTypes.Tts]: "Text to speech",
	[IBitTypes.Stt]: "Speech to text",
	[IBitTypes.Embedding]: "Text embeddings",
	[IBitTypes.ImageEmbedding]: "Image embeddings",
	[IBitTypes.ImageGeneration]: "Image generation",
	[IBitTypes.VideoGeneration]: "Video generation",
	[IBitTypes.ObjectDetection]: "Object detection",
	[IBitTypes.Forecasting]: "Forecasting",
	[IBitTypes.SystemOne]: "Decisions",
} as const;

export interface ModelQualityEstimate {
	category: keyof typeof MODEL_QUALITY_CATEGORIES;
	criteria: string;
	score: number;
	low: number;
	high: number;
	confidence: "low" | "medium";
	reasoning: string;
	sources: string[];
	updated_at: string;
}

function isQualityScore(value: unknown): value is number {
	return (
		typeof value === "number" &&
		Number.isFinite(value) &&
		value >= 0 &&
		value <= 100
	);
}

function isEvidenceSource(value: unknown): value is string {
	if (typeof value !== "string") return false;
	try {
		const url = new URL(value);
		return url.protocol === "https:" && !url.username && !url.password;
	} catch {
		return false;
	}
}

/** Quality is a catalog estimate within one task category, on a 0–100 scale. */
export function modelQualityEstimate(
	value: unknown,
	expectedCategory?: IBitTypes,
): ModelQualityEstimate | null {
	if (!value || typeof value !== "object" || Array.isArray(value)) return null;
	const estimate = value as Partial<ModelQualityEstimate>;
	if (
		!Object.hasOwn(MODEL_QUALITY_CATEGORIES, estimate.category ?? "") ||
		(expectedCategory !== undefined &&
			estimate.category !== expectedCategory) ||
		!isQualityScore(estimate.score) ||
		!isQualityScore(estimate.low) ||
		!isQualityScore(estimate.high) ||
		estimate.low > estimate.score ||
		estimate.score > estimate.high ||
		!["low", "medium"].includes(estimate.confidence ?? "") ||
		![estimate.criteria, estimate.reasoning].every(
			(text) => typeof text === "string" && text.trim(),
		) ||
		!Array.isArray(estimate.sources) ||
		estimate.sources.length === 0 ||
		!estimate.sources.every(isEvidenceSource) ||
		typeof estimate.updated_at !== "string" ||
		!Number.isFinite(Date.parse(estimate.updated_at))
	) {
		return null;
	}
	return estimate as ModelQualityEstimate;
}

export function bitQualityEstimate(bit: IBit): ModelQualityEstimate | null {
	return modelQualityEstimate(
		bit.model_evaluation?.evaluations?.quality_estimate,
		bit.type,
	);
}

/** Group categories before ranking so unrelated quality scales never compete. */
export function compareModelQuality(a: IBit, b: IBit): number {
	const aQuality = bitQualityEstimate(a);
	const bQuality = bitQualityEstimate(b);
	const categoryOrder = a.type.localeCompare(b.type);
	if (categoryOrder) return categoryOrder;
	const scoreOrder = (bQuality?.score ?? -1) - (aQuality?.score ?? -1);
	return (
		scoreOrder ||
		(a.meta?.en?.name || a.id).localeCompare(b.meta?.en?.name || b.id)
	);
}
