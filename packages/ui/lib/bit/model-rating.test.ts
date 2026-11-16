import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import { bitRating, compareRating, ratingTier } from "./model-rating";

const estimate = {
	score: 23,
	low: 18,
	high: 28,
	confidence: "low",
	reasoning: "Estimated from the model card and comparable models.",
	sources: ["https://huggingface.co/example/model"],
	updated_at: "2026-10-27",
	version: "5.0.1",
	method: "paired-version-calibration",
};

const currentVersion = {
	artificial_analysis_intelligence_index_version: "5.0.1",
	benchmark_status: "measured",
};

function bit(type: IBitTypes, evaluations: Record<string, unknown>): IBit {
	return {
		id: `${type}-model`,
		type,
		model_evaluation: {
			slug: "model",
			name: "Model",
			creator_name: "Lab",
			creator_slug: "lab",
			evaluations,
		},
	} as unknown as IBit;
}

describe("model rating", () => {
	test("tiers each scale by its own floors", () => {
		expect(
			[30, 29.9, 18, 10, 9.9].map((s) => ratingTier("intelligence", s)),
		).toEqual([1, 2, 2, 3, 4]);
		expect([80, 79, 65, 50, 49].map((s) => ratingTier("quality", s))).toEqual([
			1, 2, 2, 3, 4,
		]);
	});

	test("rates chat models by the measured index before any estimate, including zero", () => {
		for (const score of [0, 51]) {
			const rating = bitRating(
				bit(IBitTypes.Llm, {
					...currentVersion,
					artificial_analysis_intelligence_index: score,
					intelligence_estimate: estimate,
				}),
			);
			expect(rating).toMatchObject({
				kind: "intelligence",
				score,
				estimated: false,
			});
		}
	});

	test("falls back to a valid intelligence estimate with its range", () => {
		const rating = bitRating(
			bit(IBitTypes.Vlm, {
				...currentVersion,
				benchmark_status: "estimated",
				intelligence_estimate: estimate,
			}),
		);
		expect(rating).toMatchObject({
			kind: "intelligence",
			score: 23,
			estimated: true,
			tier: 2,
			low: 18,
			high: 28,
			confidence: "low",
		});
		expect(
			bitRating(
				bit(IBitTypes.Llm, {
					...currentVersion,
					benchmark_status: "estimated",
					intelligence_estimate: { ...estimate, low: 30 },
				}),
			),
		).toBeNull();
	});

	test("does not rank old versions or unversioned estimates on the current scale", () => {
		for (const version of [undefined, "4.0", "5.0", "5.0.0"]) {
			expect(
				bitRating(
					bit(IBitTypes.Llm, {
						...currentVersion,
						artificial_analysis_intelligence_index_version: version,
						artificial_analysis_intelligence_index: 55.1,
						intelligence_estimate: estimate,
					}),
				),
			).toBeNull();
		}
		for (const invalidEstimate of [
			{ ...estimate, version: undefined },
			{ ...estimate, version: "4.0" },
			{ ...estimate, method: undefined },
		]) {
			expect(
				bitRating(
					bit(IBitTypes.Llm, {
						...currentVersion,
						benchmark_status: "estimated",
						intelligence_estimate: invalidEstimate,
					}),
				),
			).toBeNull();
		}
	});

	test("rates every other type by its own category quality estimate only", () => {
		const quality = {
			...estimate,
			category: IBitTypes.Tts,
			criteria: "Naturalness and pronunciation.",
			score: 84,
			low: 78,
			high: 89,
		};
		expect(
			bitRating(bit(IBitTypes.Tts, { quality_estimate: quality })),
		).toMatchObject({
			kind: "quality",
			score: 84,
			tier: 1,
			criteria: quality.criteria,
		});
		expect(
			bitRating(bit(IBitTypes.Stt, { quality_estimate: quality })),
		).toBeNull();
		expect(
			bitRating(
				bit(IBitTypes.Tts, { artificial_analysis_intelligence_index: 50 }),
			),
		).toBeNull();
	});

	test("sorts rated models first, highest score first", () => {
		const models = [
			bit(IBitTypes.Llm, {}),
			bit(IBitTypes.Llm, {
				...currentVersion,
				artificial_analysis_intelligence_index: 20,
			}),
			bit(IBitTypes.Llm, {
				...currentVersion,
				artificial_analysis_intelligence_index: 40,
			}),
		];
		const scores = [...models]
			.sort(compareRating)
			.map((model) => bitRating(model)?.score ?? null);
		expect(scores).toEqual([40, 20, null]);
	});

	test("groups separate quality categories while comparing chat and vision on the same index", () => {
		const qualityBit = (type: IBitTypes, score: number) =>
			bit(type, {
				quality_estimate: {
					...estimate,
					category: type,
					criteria: "Quality within this task category.",
					score,
					low: score,
					high: score,
				},
			});
		const models = [
			qualityBit(IBitTypes.Tts, 95),
			bit(IBitTypes.Llm, {}),
			qualityBit(IBitTypes.Stt, 10),
			bit(IBitTypes.Llm, {
				...currentVersion,
				artificial_analysis_intelligence_index: 0,
			}),
			qualityBit(IBitTypes.Tts, 0),
			bit(IBitTypes.Tts, {}),
			bit(IBitTypes.Vlm, {
				...currentVersion,
				artificial_analysis_intelligence_index: 40,
			}),
			qualityBit(IBitTypes.Stt, 20),
		];
		expect(
			models
				.sort(compareRating)
				.map((model) => [model.type, bitRating(model)?.score ?? null]),
		).toEqual([
			[IBitTypes.Vlm, 40],
			[IBitTypes.Llm, 0],
			[IBitTypes.Llm, null],
			[IBitTypes.Stt, 20],
			[IBitTypes.Stt, 10],
			[IBitTypes.Tts, 95],
			[IBitTypes.Tts, 0],
			[IBitTypes.Tts, null],
		]);
	});
});
