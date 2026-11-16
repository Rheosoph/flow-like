import type {
	ModelClassification,
	ModelClassificationEstimates,
} from "./types";

const VERIFIED_AT = "2026-11-16";
const MODEL_URL = "https://openrouter.ai/qwen/qwen4-max";
const ENDPOINTS_URL =
	"https://openrouter.ai/api/v1/models/qwen/qwen4-max/endpoints";
const BENCHMARK_URL = "https://artificialanalysis.ai/models/qwen4-max";

const classification: ModelClassification = {
	coding: 0.375,
	cost: 0.54,
	creativity: 0.37,
	factuality: 0.576,
	function_calling: 0.726,
	multilinguality: 0.95,
	openness: 0.11,
	reasoning: 0.371,
	safety: 0,
	speed: 0.49,
};

// Catalog normalization is separate from the raw, independently measured scores.
const classificationEstimates: ModelClassificationEstimates = {
	creativity: {
		value: classification.creativity,
		confidence: "low",
		reasoning:
			"Conservative family transfer from Qwen3.8-27B's 37% estimated pairwise writing win chance on the November 16 leaderboard. The donor has a different architecture and size, and no Qwen4 Max writing result is available. This provisional catalog rating is not a measured writing score for Qwen4 Max.",
		sources: ["https://github.com/lechmazur/writing", MODEL_URL],
		updated_at: VERIFIED_AT,
	},
	multilinguality: {
		value: classification.multilinguality,
		confidence: "low",
		reasoning:
			"Family transfer from Qwen3.5 397B A17B's normalized multilingual index of 0.951177 in the September 12 catalog benchmark snapshot, rounded to two decimals. No Qwen4 Max language benchmark is available; individual languages may differ substantially.",
		sources: [
			"https://artificialanalysis.ai/models/qwen3-5-397b-a17b",
			"https://artificialanalysis.ai/models/multilingual",
		],
		updated_at: VERIFIED_AT,
	},
	speed: {
		value: classification.speed,
		confidence: "low",
		reasoning:
			"Family transfer from Qwen3.8 Max's 41.728 output tokens per second in the September 12 catalog benchmark snapshot, using the existing log(speed)/log(2148.933) normalization. Qwen4 Max has no published endpoint throughput measurement. Hardware, provider load and reasoning effort can change actual latency substantially.",
		sources: [
			"https://artificialanalysis.ai/models/qwen3-8-max",
			ENDPOINTS_URL,
		],
		updated_at: VERIFIED_AT,
	},
	coding: {
		value: classification.coding,
		confidence: "medium",
		reasoning:
			"Mean of Terminal-Bench 4.0 (0.484848) and FrontierCode-AA 2.0 (0.264535) raw task fractions. This is a catalog capability summary, separate from the weighted AA Intelligence Index.",
		sources: [BENCHMARK_URL],
		updated_at: VERIFIED_AT,
	},
	function_calling: {
		value: classification.function_calling,
		confidence: "medium",
		reasoning:
			"AutomationBench-AA raw task fraction (0.725612), rounded to three decimals.",
		sources: [BENCHMARK_URL],
		updated_at: VERIFIED_AT,
	},
	reasoning: {
		value: classification.reasoning,
		confidence: "medium",
		reasoning:
			"Mean of HLE-Diamond (0.449), CritPt v2 Hard (0.43) and Terminal-Bench Science 0.1.0 (0.233333) raw task fractions, rounded to three decimals.",
		sources: [BENCHMARK_URL],
		updated_at: VERIFIED_AT,
	},
	openness: {
		value: classification.openness,
		confidence: "medium",
		reasoning:
			"The catalog's proprietary-license fallback (11.111/100), rounded to two decimals. Artificial Analysis identifies this model as proprietary with no available weights; this is not a measured openness index.",
		sources: [BENCHMARK_URL],
		updated_at: VERIFIED_AT,
	},
};

/** Verified catalog seed. Refresh provider limits and benchmark sources before changing it. */
export const QWEN4_MAX = {
	slug: "qwen4-max",
	name: "Qwen4 Max",
	type: "VLM",
	releaseDate: "2026-11-16",
	creatorName: "Alibaba",
	creatorSlug: "alibaba",
	authors: ["Alibaba", "Qwen"],
	hub: "api.flow-like.com",
	repository: MODEL_URL,
	license: "Proprietary",
	evaluations: {
		artificial_analysis_intelligence_index: 42.8914543287565,
		artificial_analysis_intelligence_index_version: "5.0.1",
		benchmark_status: "measured",
		terminalbench_v4_0: 0.484848484848485,
		frontiercode_v2_0: 0.264534595959596,
		terminalbench_science_v0_1: 0.233333333333333,
		gdp_pdf_all_pass: 0.262,
		aa_briefcase_elo: 1686.07,
		artificial_analysis_omniscience: 27.7,
		automationbench_aa: 0.7256119376604481,
		artificial_analysis_omniscience_accuracy: 0.437,
		artificial_analysis_omniscience_hallucination_rate: 0.2841918294849023,
		// Preserve newer benchmark names instead of treating them as older task sets.
		hle_diamond: 0.449,
		critpt_agentic_hard: 0.43,
		benchmark_source_url: BENCHMARK_URL,
		benchmark_verified_at: VERIFIED_AT,
		benchmark_reasoning_effort: "xhigh",
	},
	pricing: {
		price_1m_input_tokens: 1.5,
		price_1m_output_tokens: 4.5,
		price_1m_blended_3_to_1: 2.25,
	},
	medianOutputTokensPerSecond: null,
	medianTimeToFirstTokenSeconds: null,
	medianTimeToFirstAnswerToken: null,
	parameters: {
		context_length: 1_000_000,
		// Use the live endpoint limit; the marketing FAQ lists a larger limit.
		max_output_tokens: 131_072,
		provider: {
			api_surface: "ChatCompletions",
			provider_name: "hosted:openrouter",
			model_id: "qwen/qwen4-max",
			version: null,
			params: {
				endpoint: "https://api.flow-like.com/api/v1",
				tier: "PRO",
			},
		},
		model_classification: classification,
		model_classification_estimates: classificationEstimates,
		classification_estimated_fields: ["creativity", "multilinguality", "speed"],
		classification_unavailable_fields: ["safety"],
		classification_method: {
			version: "aa-v5.0.1",
			factuality:
				"Mean of AA-Omniscience v1.1 accuracy and one minus hallucination rate, rounded to three decimals.",
			safety:
				"Unavailable. The AA intelligence suite does not measure harmful-content safety. Numeric zero is a runtime compatibility value, not a measurement.",
			cost: "Existing logarithmic cost normalization for a $2.25 blended 3:1 input/output rate, using the September 12 catalog price range of $0.028 to $262.50 per million tokens.",
			family_estimates:
				"Creativity, multilinguality and speed use provisional ratings transferred from earlier Qwen models. Their metadata identifies the donor, source date and limits; none is a Qwen4 Max measurement.",
			verified_at: VERIFIED_AT,
			sources: [BENCHMARK_URL, ENDPOINTS_URL],
		},
		benchmark_notes:
			"Artificial Analysis v5.0.1 measured Qwen4 Max at xhigh reasoning effort; the OpenRouter default is high. Creativity, multilinguality and speed are low-confidence estimates transferred from earlier Qwen models. Coding, reasoning and tool-use ratings summarize current raw task fractions. The AA index primarily measures English text and agent tasks.",
		capabilities: {
			input_modalities: ["text", "image", "video"],
			output_modalities: ["text"],
			function_calling: true,
			structured_outputs: true,
			streaming: true,
			reasoning: true,
			reasoning_efforts: ["xhigh", "high", "low", "none"],
			default_reasoning_effort: "high",
			sources: [MODEL_URL, ENDPOINTS_URL],
			verified_at: VERIFIED_AT,
		},
		pricing: {
			input_micro_usd_per_million_tokens: 1_500_000,
			output_micro_usd_per_million_tokens: 4_500_000,
			request_micro_usd: 0,
		},
		pricing_source: {
			basis:
				"Standard uncached token rates from the sole advertised Alibaba endpoint. No long-context override is advertised. Cache creation and cache-read rates are listed separately by the provider.",
			url: ENDPOINTS_URL,
			verified_at: VERIFIED_AT,
		},
	},
	meta: {
		name: "Qwen4 Max",
		description:
			"Reason over documents, images and video with Qwen4 Max, a hosted Alibaba model with tool calling and a one-million-token context window.",
		longDescription:
			"Qwen4 Max accepts text, images and video and returns text. Use it for document analysis, coding tasks and workflows that call tools. Structured outputs are supported.\n\nOpenRouter enables reasoning by default at high effort. Everyday results and latency can differ from the evaluated configuration shown in the benchmark details.\n\nThe catalog uses OpenRouter's live endpoint limit of 131,072 output tokens and a total context window of 1,000,000 tokens. Standard rates are $1.50 per million input tokens and $4.50 per million output tokens. This is a proprietary hosted model.",
		tags: [
			"qwen",
			"alibaba",
			"hosted",
			"vision",
			"video-understanding",
			"reasoning",
			"coding",
			"function-calling",
			"structured-outputs",
			"long-context",
			"openrouter",
			"pro",
		],
		useCase: "Analyze documents and visual content, write code and call tools",
		icon: "https://artificialanalysis.ai/img/logos/alibaba.svg",
		website: MODEL_URL,
		docsUrl: MODEL_URL,
	},
};
