import { describe, expect, test } from "bun:test";
import { computeGlobalMaxes } from "./fetch";
import { catalogBlendedTokenPrice, computeClassification } from "./normalize";
import type {
	AAEvaluations,
	AAModel,
	ModelClassification,
	ModelClassificationEstimates,
} from "./types";

function model(evaluations: Partial<AAEvaluations>): AAModel {
	return {
		id: "test-model",
		name: "Test model",
		slug: "test-model",
		release_date: null,
		model_creator: { id: "test", name: "Test", slug: "test" },
		evaluations: {
			artificial_analysis_intelligence_index: null,
			artificial_analysis_coding_index: null,
			artificial_analysis_math_index: null,
			artificial_analysis_multilingual_index_normalized: null,
			artificial_analysis_openness_index: null,
			artificial_analysis_omniscience_index_normalized: null,
			artificial_analysis_omniscience_accuracy: null,
			artificial_analysis_omniscience_hallucination_rate: null,
			writing_benchmark_mean_score: null,
			writing_benchmark_version: null,
			writing_benchmark_match_type: null,
			writing_benchmark_source_model_name: null,
			writing_benchmark_source_model_slug: null,
			writing_benchmark_similarity: null,
			writing_benchmark_similarity_tier: null,
			mmlu_pro: null,
			gpqa: null,
			hle: null,
			livecodebench: null,
			scicode: null,
			math_500: null,
			aime: null,
			aime_25: null,
			ifbench: null,
			lcr: null,
			terminalbench_hard: null,
			tau2: null,
			...evaluations,
		},
		pricing: {
			price_1m_blended_3_to_1: null,
			price_1m_input_tokens: null,
			price_1m_output_tokens: null,
		},
		median_output_tokens_per_second: null,
		median_time_to_first_token_seconds: null,
		median_time_to_first_answer_token: null,
	};
}

const reference = model({
	scicode: 0.8,
	terminalbench_v4_0: 0.8,
	terminalbench_v2_1: 0.9,
	terminalbench_hard: 1,
	automationbench_aa: 0.8,
	tau_banking: 0.8,
	tau2: 0.9,
	ifbench: 1,
	mmlu_pro: 1,
	gpqa: 1,
});

function classify(
	evaluations: Partial<AAEvaluations>,
	existing?: ModelClassification,
	estimates?: ModelClassificationEstimates,
) {
	const subject = model(evaluations);
	return computeClassification(
		subject,
		computeGlobalMaxes([reference, subject]),
		[],
		false,
		existing,
		undefined,
		undefined,
		estimates,
	);
}

describe("current benchmark capability ratings", () => {
	test("converts catalog billing units and rejects pricing without comparable token rates", () => {
		expect(
			catalogBlendedTokenPrice({
				pricing: {
					input_micro_usd_per_million_tokens: 3_600_000,
					output_micro_usd_per_million_tokens: 18_000_000,
					request_micro_usd: 0,
				},
			}),
		).toBe(7.2);
		for (const pricing of [
			{ input_micro_usd_per_million_tokens: 100_000 },
			{
				input_micro_usd_per_million_tokens: -1,
				output_micro_usd_per_million_tokens: 0,
			},
			{
				input_micro_usd_per_million_tokens: "100000",
				output_micro_usd_per_million_tokens: 0,
			},
			{
				input_micro_usd_per_million_tokens: Number.NaN,
				output_micro_usd_per_million_tokens: 0,
			},
			{
				input_micro_usd_per_million_tokens: 100_000,
				output_micro_usd_per_million_tokens: Number.POSITIVE_INFINITY,
			},
			{
				input_micro_usd_per_million_tokens: 100_000,
				output_micro_usd_per_million_tokens: 0,
				request_micro_usd: 10,
			},
		])
			expect(catalogBlendedTokenPrice({ pricing })).toBeUndefined();
		expect(catalogBlendedTokenPrice(null)).toBeUndefined();
	});

	test("rates actual billed prices and recognizes explicitly free token usage", () => {
		const subject = model({});
		subject.pricing.price_1m_blended_3_to_1 = 4;
		const rating = (price?: number) =>
			computeClassification(
				subject,
				computeGlobalMaxes([subject]),
				[0.1, 4, 20],
				false,
				undefined,
				undefined,
				price,
			);
		expect(rating(7.2).classification.cost).toBeLessThan(
			rating().classification.cost,
		);
		const free = catalogBlendedTokenPrice({
			pricing: {
				input_micro_usd_per_million_tokens: 0,
				output_micro_usd_per_million_tokens: 0,
			},
		});
		expect(free).toBe(0);
		expect(rating(free).classification.cost).toBe(1);
		expect(rating(free).missingFields).not.toContain("cost (no pricing data)");
		subject.pricing.price_1m_blended_3_to_1 = 0;
		expect(rating().missingFields).toContain("cost (no pricing data)");
	});

	test("uses current coding benchmarks when the legacy fields are absent", () => {
		const { classification } = classify({
			scicode: 0.4,
			terminalbench_v4_0: 0.4,
		});
		expect(classification.coding).toBe(0.5);
	});

	test("does not double-count Terminal-Bench versions or reuse their maxima", () => {
		const { classification } = classify({
			scicode: 0.4,
			terminalbench_v4_0: 0.4,
			terminalbench_v2_1: 0.9,
			terminalbench_hard: 1,
		});
		expect(classification.coding).toBe(0.5);
		expect(classify({ terminalbench_v2_1: 0.45 }).classification.coding).toBe(
			0.5,
		);
		expect(classify({ terminalbench_hard: 0.25 }).classification.coding).toBe(
			0.25,
		);
	});

	test("uses application tool workflows for function calling, independently of coding", () => {
		const { classification } = classify({
			automationbench_aa: 0.4,
			tau2: 0.9,
			ifbench: 1,
			terminalbench_v4_0: 0.8,
		});
		expect(classification.function_calling).toBe(0.5);
		expect(classification.coding).toBe(1);
		expect(classify({ tau_banking: 0.2 }).classification.function_calling).toBe(
			0.25,
		);
		expect(
			classify({ terminalbench_v4_0: 0.8 }).classification.function_calling,
		).toBe(0);
	});

	test("factuality uses knowledge reliability rather than overall intelligence", () => {
		const { classification } = classify({
			artificial_analysis_intelligence_index: 100,
			artificial_analysis_omniscience_accuracy: 0.4,
			artificial_analysis_omniscience_hallucination_rate: 0.2,
		});
		expect(classification.factuality).toBe(0.6);
		expect(
			classify({ artificial_analysis_intelligence_index: 100 }).classification
				.factuality,
		).toBe(0);
		expect(
			classify({ mmlu_pro: 0.5, gpqa: 0.7 }).classification.factuality,
		).toBe(0.6);
	});

	test("keeps measured zeros and only preserves old ratings when data is missing", () => {
		const existing = Object.fromEntries(
			Object.keys(classify({}).classification).map((key) => [key, 0.9]),
		) as unknown as ModelClassification;
		const { classification } = classify(
			{
				scicode: 0,
				terminalbench_v4_0: 0,
				terminalbench_v2_1: 0.9,
				automationbench_aa: 0,
				artificial_analysis_omniscience_accuracy: 0,
				artificial_analysis_omniscience_hallucination_rate: 1,
				artificial_analysis_multilingual_index: 0,
				artificial_analysis_openness_index: 0,
				writing_benchmark_estimated_win_chance: 0,
			},
			existing,
		);
		expect(classification.coding).toBe(0);
		expect(classification.function_calling).toBe(0);
		expect(classification.factuality).toBe(0);
		expect(classification.safety).toBe(0);
		expect(classification.multilinguality).toBe(0);
		expect(classification.openness).toBe(0);
		expect(classification.creativity).toBe(0);
		expect(classification.reasoning).toBe(0.9);
	});

	test("uses raw multilingual accuracy when the normalized dataset is absent", () => {
		expect(
			classify({ artificial_analysis_multilingual_index: 0.63 }).classification
				.multilinguality,
		).toBe(0.63);
		expect(
			classify({
				artificial_analysis_multilingual_index: 0.63,
				artificial_analysis_multilingual_index_normalized: 0.75,
			}).classification.multilinguality,
		).toBe(0.75);
	});

	test("uses partial observed results and rejects invalid measurements", () => {
		const { classification, missingFields } = classify({ scicode: 0.4 });
		expect(classification.coding).toBe(0.5);
		expect(missingFields).toContain("coding (partial benchmarks)");
		const invalid = model({
			scicode: Number.POSITIVE_INFINITY,
			gpqa: Number.NaN,
		});
		const maxes = computeGlobalMaxes([invalid, reference]);
		expect(maxes.scicode).toBe(0.8);
		expect(maxes.gpqa).toBe(1);
		expect(classify({ scicode: Number.NaN }).classification.coding).toBe(0);
	});

	test("retains researched ratings and their evidence until measured results arrive", () => {
		const existing = {
			...classify({}).classification,
			coding: 0.5,
			creativity: 0.7,
			multilinguality: 0,
		};
		const evidence = {
			confidence: "low" as const,
			reasoning: "Estimated from comparable models.",
			sources: ["https://example.com/model-card"],
			updated_at: "2026-10-27",
		};
		const estimates = {
			coding: { ...evidence, value: existing.coding },
			creativity: { ...evidence, value: existing.creativity },
			multilinguality: { ...evidence, value: existing.multilinguality },
		};
		const missing = classify({}, existing, estimates);
		expect(missing.classification).toEqual(existing);
		expect(missing.classificationEstimates).toEqual(estimates);

		const similarity = classify(
			{
				writing_benchmark_creativity_estimate: 0.4,
				writing_benchmark_match_type: "similarity",
			},
			existing,
			estimates,
		);
		expect(similarity.classification.creativity).toBe(0.7);
		expect(similarity.classificationEstimates.creativity).toEqual(
			estimates.creativity,
		);

		const measured = classify(
			{
				scicode: 0.4,
				writing_benchmark_estimated_win_chance: 0,
				writing_benchmark_match_type: "direct",
			},
			existing,
			estimates,
		);
		expect(measured.classification.coding).toBe(0.5);
		expect(measured.classification.creativity).toBe(0);
		expect(measured.classificationEstimates).toEqual({
			multilinguality: estimates.multilinguality,
		});
	});

	test("does not carry stale attribution onto an unrelated manual rating", () => {
		const existing = { ...classify({}).classification, coding: 0.8 };
		const result = classify({}, existing, {
			coding: {
				value: 0.5,
				confidence: "low",
				reasoning: "Old estimate.",
				sources: [],
				updated_at: "2026-10-27",
			},
		});
		expect(result.classification.coding).toBe(0.8);
		expect(result.classificationEstimates).toEqual({});
	});
});
