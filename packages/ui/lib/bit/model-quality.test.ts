import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	MODEL_QUALITY_CATEGORIES,
	compareModelQuality,
	modelQualityEstimate,
} from "./model-quality";

const quality = {
	category: IBitTypes.Tts,
	criteria: "Speech naturalness and intelligibility.",
	score: 85,
	low: 75,
	high: 90,
	confidence: "low",
	reasoning:
		"Estimated from the model card; local inference was not benchmarked.",
	sources: ["https://huggingface.co/example/voice-model"],
	updated_at: "2026-11-16",
};

describe("model quality estimates", () => {
	test("accepts every supported model category without treating LLM intelligence as quality", () => {
		for (const category of Object.keys(MODEL_QUALITY_CATEGORIES)) {
			expect(modelQualityEstimate({ ...quality, category })?.score).toBe(85);
		}
		expect(
			modelQualityEstimate({ ...quality, category: IBitTypes.Llm }),
		).toBeNull();
		expect(
			modelQualityEstimate({ ...quality, category: "constructor" }),
		).toBeNull();
		expect(modelQualityEstimate(quality, IBitTypes.Stt)).toBeNull();
	});

	test("rejects invalid scores, ranges and missing provenance", () => {
		for (const invalid of [
			null,
			[],
			{},
			{ ...quality, score: Number.NaN },
			{ ...quality, score: Number.POSITIVE_INFINITY },
			{ ...quality, low: -1 },
			{ ...quality, low: 86 },
			{ ...quality, high: 84 },
			{ ...quality, high: 101 },
			{ ...quality, score: "85" },
			{ ...quality, criteria: " " },
			{ ...quality, reasoning: null },
			{ ...quality, confidence: "high" },
			{ ...quality, sources: [] },
			{ ...quality, sources: [null] },
			{ ...quality, sources: [""] },
			{ ...quality, sources: ["javascript:alert(1)"] },
			{ ...quality, sources: ["https://user:secret@example.com/private"] },
			{ ...quality, updated_at: "unknown" },
		]) {
			expect(modelQualityEstimate(invalid)).toBeNull();
		}
	});

	test("ranks scores within categories and leaves unscored models after zero estimates", () => {
		function bit(id: string, type: IBitTypes, score?: number): IBit {
			return {
				id,
				type,
				meta: {},
				model_evaluation: {
					evaluations: {
						quality_estimate:
							score === undefined
								? undefined
								: { ...quality, category: type, score, low: 0, high: 100 },
					},
				},
			} as unknown as IBit;
		}
		const models = [
			bit("unscored", IBitTypes.Tts),
			bit("tts-high", IBitTypes.Tts, 95),
			bit("stt-low", IBitTypes.Stt, 40),
			bit("tts-zero", IBitTypes.Tts, 0),
			bit("stt-high", IBitTypes.Stt, 80),
		];
		expect(models.sort(compareModelQuality).map((model) => model.id)).toEqual([
			"stt-high",
			"stt-low",
			"tts-high",
			"tts-zero",
			"unscored",
		]);
	});
});
