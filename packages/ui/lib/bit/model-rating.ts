import { type IBit, IBitTypes } from "../schema/bit/bit";
import { bitQualityEstimate } from "./model-quality";

export interface IntelligenceEstimate {
	score: number;
	low: number;
	high: number;
	confidence: "low" | "medium";
	reasoning: string;
	sources: string[];
	updated_at: string;
	version?: string;
	method?: string;
}

export const AA_INTELLIGENCE_VERSION = "5.0.1";

interface IntelligenceEvaluations {
	artificial_analysis_intelligence_index?: unknown;
	artificial_analysis_intelligence_index_version?: unknown;
	benchmark_status?: unknown;
	intelligence_estimate?: unknown;
}

export function isScore(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

export function intelligenceEstimate(
	value: unknown,
): IntelligenceEstimate | null {
	if (!value || typeof value !== "object") return null;
	const estimate = value as Partial<IntelligenceEstimate>;
	if (
		!isScore(estimate.score) ||
		!isScore(estimate.low) ||
		!isScore(estimate.high) ||
		estimate.high > 100 ||
		estimate.low > estimate.score ||
		estimate.score > estimate.high ||
		!["low", "medium"].includes(estimate.confidence ?? "") ||
		typeof estimate.reasoning !== "string" ||
		!estimate.reasoning.trim() ||
		!Array.isArray(estimate.sources) ||
		!estimate.sources.every((source) => typeof source === "string") ||
		typeof estimate.updated_at !== "string" ||
		!Number.isFinite(Date.parse(estimate.updated_at))
	) {
		return null;
	}
	return estimate as IntelligenceEstimate;
}

export function currentIntelligenceIndex(
	evals?: IntelligenceEvaluations | null,
): number | null {
	const index = evals?.artificial_analysis_intelligence_index;
	return evals?.artificial_analysis_intelligence_index_version ===
		AA_INTELLIGENCE_VERSION &&
		evals.benchmark_status === "measured" &&
		isScore(index) &&
		index <= 100
		? index
		: null;
}

export function currentIntelligenceEstimate(
	evals?: IntelligenceEvaluations | null,
): IntelligenceEstimate | null {
	if (
		evals?.artificial_analysis_intelligence_index_version !==
			AA_INTELLIGENCE_VERSION ||
		evals.benchmark_status !== "estimated"
	) {
		return null;
	}
	const estimate = intelligenceEstimate(evals.intelligence_estimate);
	return estimate?.version === AA_INTELLIGENCE_VERSION &&
		estimate.method === "paired-version-calibration"
		? estimate
		: null;
}

export type RatingKind = "intelligence" | "quality";
export type RatingTier = 1 | 2 | 3 | 4;

export interface ModelRating {
	kind: RatingKind;
	score: number;
	estimated: boolean;
	tier: RatingTier;
	low?: number;
	high?: number;
	confidence?: "low" | "medium";
	criteria?: string;
	reasoning?: string;
	sources: string[];
	updatedAt?: string;
}

/**
 * Lowest score of the top, strong and fair tiers. The intelligence index and the
 * 0–100 per-category quality estimates sit on different scales, so each has its own.
 */
export const RATING_FLOORS: Record<
	RatingKind,
	readonly [top: number, strong: number, fair: number]
> = {
	intelligence: [30, 18, 10],
	quality: [80, 65, 50],
};

export function ratingTier(kind: RatingKind, score: number): RatingTier {
	const [top, strong, fair] = RATING_FLOORS[kind];
	if (score >= top) return 1;
	if (score >= strong) return 2;
	if (score >= fair) return 3;
	return 4;
}

const INTELLIGENCE_TYPES = new Set<IBitTypes>([IBitTypes.Llm, IBitTypes.Vlm]);

/**
 * Chat models use the current intelligence index or an explicit estimate on that
 * version's scale. Other types use their category quality estimate.
 */
export function bitRating(bit: IBit): ModelRating | null {
	if (INTELLIGENCE_TYPES.has(bit.type)) {
		const evaluations = bit.model_evaluation?.evaluations as
			| Record<string, unknown>
			| null
			| undefined;
		const measured = currentIntelligenceIndex(evaluations);
		if (measured !== null) {
			return {
				kind: "intelligence",
				score: measured,
				estimated: false,
				tier: ratingTier("intelligence", measured),
				sources: [],
			};
		}
		const estimate = currentIntelligenceEstimate(evaluations);
		if (!estimate) return null;
		return {
			kind: "intelligence",
			score: estimate.score,
			estimated: true,
			tier: ratingTier("intelligence", estimate.score),
			low: estimate.low,
			high: estimate.high,
			confidence: estimate.confidence,
			reasoning: estimate.reasoning,
			sources: estimate.sources,
			updatedAt: estimate.updated_at,
		};
	}
	const quality = bitQualityEstimate(bit);
	if (!quality) return null;
	return {
		kind: "quality",
		score: quality.score,
		estimated: true,
		tier: ratingTier("quality", quality.score),
		low: quality.low,
		high: quality.high,
		confidence: quality.confidence,
		criteria: quality.criteria,
		reasoning: quality.reasoning,
		sources: quality.sources,
		updatedAt: quality.updated_at,
	};
}

/** Group comparable categories, then sort higher first and unrated last. */
export function compareRating(a: IBit, b: IBit): number {
	const categoryA = INTELLIGENCE_TYPES.has(a.type) ? IBitTypes.Llm : a.type;
	const categoryB = INTELLIGENCE_TYPES.has(b.type) ? IBitTypes.Llm : b.type;
	if (categoryA !== categoryB) return categoryA.localeCompare(categoryB);
	return (bitRating(b)?.score ?? -1) - (bitRating(a)?.score ?? -1);
}
