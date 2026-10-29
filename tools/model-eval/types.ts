/** Shared types for the model-eval pipeline */

export function isCatalogEstimateSlug(slug: string): boolean {
	return slug.startsWith("flow-like-estimate-");
}

export type AAOpenSourceCategorization =
	| "Open Weights (License Required for Commercial Use)"
	| "Open Weights (Permissive License)"
	| "Proprietary";

export type WritingBenchmarkVersion = "v2" | "v3" | "v4" | "pairwise";

export type CreativityBenchmarkMatchType = "direct" | "alias" | "similarity";

export type CreativitySimilarityTier =
	| "same_creator_category"
	| "same_creator"
	| "same_category";

export interface AAModelCreator {
	id: string;
	name: string;
	slug: string;
}

export interface AAEvaluations {
	artificial_analysis_intelligence_index: number | null;
	artificial_analysis_coding_index: number | null;
	artificial_analysis_math_index: number | null;
	artificial_analysis_multilingual_index_normalized: number | null;
	artificial_analysis_multilingual_index?: number | null;
	artificial_analysis_openness_index: number | null;
	artificial_analysis_omniscience_index_normalized: number | null;
	artificial_analysis_omniscience_accuracy: number | null;
	artificial_analysis_omniscience_hallucination_rate: number | null;
	writing_benchmark_mean_score: number | null;
	writing_benchmark_comparison_score?: number | null;
	writing_benchmark_estimated_win_chance?: number | null;
	writing_benchmark_creativity_estimate?: number | null;
	writing_benchmark_source_url?: string | null;
	writing_benchmark_version: WritingBenchmarkVersion | null;
	writing_benchmark_match_type: CreativityBenchmarkMatchType | null;
	writing_benchmark_source_model_name: string | null;
	writing_benchmark_source_model_slug: string | null;
	writing_benchmark_similarity: number | null;
	writing_benchmark_similarity_tier: CreativitySimilarityTier | null;
	mmlu_pro: number | null;
	gpqa: number | null;
	hle: number | null;
	livecodebench: number | null;
	scicode: number | null;
	math_500: number | null;
	aime: number | null;
	aime_25: number | null;
	ifbench: number | null;
	lcr: number | null;
	terminalbench_hard: number | null;
	terminalbench_v2_1?: number | null;
	terminalbench_v4_0?: number | null;
	tau2: number | null;
	tau_banking?: number | null;
	automationbench_aa?: number | null;
}

export interface AAPricing {
	price_1m_blended_3_to_1: number | null;
	price_1m_input_tokens: number | null;
	price_1m_output_tokens: number | null;
}

export interface AAModel {
	id: string;
	name: string;
	slug: string;
	release_date: string | null;
	model_creator: AAModelCreator;
	evaluations: AAEvaluations;
	open_source_categorization?: AAOpenSourceCategorization | null;
	pricing: AAPricing;
	median_output_tokens_per_second: number | null;
	median_time_to_first_token_seconds: number | null;
	median_time_to_first_answer_token: number | null;
}

export interface AAApiResponse {
	status: number;
	prompt_options: { parallel_queries: number; prompt_length: number };
	data: AAModel[];
}

export interface GlobalMaxes {
	coding_index: number;
	livecodebench: number;
	scicode: number;
	terminalbench_hard: number;
	terminalbench_v2_1: number;
	terminalbench_v4_0: number;
	math_index: number;
	aime: number;
	aime_25: number;
	math_500: number;
	hle: number;
	gpqa: number;
	lcr: number;
	mmlu_pro: number;
	intelligence_index: number;
	tau2: number;
	tau_banking: number;
	automationbench_aa: number;
	ifbench: number;
	price_1m_blended_3_to_1: number;
	median_output_tokens_per_second: number;
}

export interface ModelClassification {
	coding: number;
	cost: number;
	creativity: number;
	factuality: number;
	function_calling: number;
	multilinguality: number;
	openness: number;
	reasoning: number;
	safety: number;
	speed: number;
}

export interface ModelClassificationEstimate {
	value: number;
	confidence: "low" | "medium";
	reasoning: string;
	sources: string[];
	updated_at: string;
}

export type ModelClassificationEstimates = Partial<
	Record<keyof ModelClassification, ModelClassificationEstimate>
>;

export interface TodoEntry {
	modelSlug: string;
	modelName: string;
	missingFields: string[];
}

export interface ResultsFile {
	fetchedAt: string;
	modelCount: number;
	globalMaxes: GlobalMaxes;
	data: AAModel[];
}
