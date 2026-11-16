import { describe, expect, test } from "bun:test";
import { computeTier } from "./db";
import { catalogBlendedTokenPrice } from "./normalize";
import { QWEN4_MAX } from "./qwen4-max";

describe("Qwen4 Max catalog seed", () => {
	test("keeps admission prices and provider limits consistent", () => {
		expect(catalogBlendedTokenPrice(QWEN4_MAX.parameters)).toBe(
			QWEN4_MAX.pricing.price_1m_blended_3_to_1,
		);
		expect(
			computeTier(
				QWEN4_MAX.parameters,
				QWEN4_MAX.parameters.model_classification.cost,
			)?.newTier,
		).toBe(QWEN4_MAX.parameters.provider.params.tier);
		expect(QWEN4_MAX.parameters.max_output_tokens).toBe(131_072);
		expect(QWEN4_MAX.parameters.capabilities.input_modalities).toEqual([
			"text",
			"image",
			"video",
		]);
		expect(QWEN4_MAX.type).toBe("VLM");
	});

	test("preserves measured provenance and newer benchmark identities", () => {
		expect(
			QWEN4_MAX.evaluations.artificial_analysis_intelligence_index,
		).toBeCloseTo(42.8914543287565);
		expect(QWEN4_MAX.evaluations.benchmark_reasoning_effort).toBe("xhigh");
		expect(
			QWEN4_MAX.evaluations.artificial_analysis_intelligence_index_version,
		).toBe("5.0.1");
		expect(QWEN4_MAX.evaluations.benchmark_status).toBe("measured");
		expect(QWEN4_MAX.evaluations).not.toHaveProperty("hle");
		expect(QWEN4_MAX.evaluations).not.toHaveProperty("intelligence_estimate");
		expect(QWEN4_MAX.medianOutputTokensPerSecond).toBeNull();
	});

	test("labels transferred ratings and keeps every value in range", () => {
		const params = QWEN4_MAX.parameters;
		for (const value of Object.values(params.model_classification)) {
			expect(Number.isFinite(value)).toBe(true);
			expect(value).toBeGreaterThanOrEqual(0);
			expect(value).toBeLessThanOrEqual(1);
		}
		for (const field of ["creativity", "multilinguality", "speed"] as const) {
			const estimate = params.model_classification_estimates[field];
			expect(estimate?.value).toBe(params.model_classification[field]);
			expect(estimate?.confidence).toBe("low");
			expect(estimate?.sources.length).toBeGreaterThan(0);
			expect(estimate?.updated_at).toBe("2026-11-16");
		}
	});
});
