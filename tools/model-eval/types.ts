/** Shared types for the model-eval pipeline */

export interface AAModelCreator {
	id: string;
	name: string;
	slug: string;
}

export interface AAEvaluations {
	artificial_analysis_intelligence_index: number | null;
	artificial_analysis_coding_index: number | null;
	artificial_analysis_math_index: number | null;
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
	tau2: number | null;
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
