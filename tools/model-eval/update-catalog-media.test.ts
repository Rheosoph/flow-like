import { describe, expect, test } from "bun:test";
import { qualityCoverage } from "./update-catalog-media";

const voice = {
	id: "voice",
	type: "Tts",
	meta: { en: { name: "Voice", description: "Voice model" } },
	model_evaluation: {
		evaluations: {
			quality_estimate: {
				category: "Tts",
				score: 80,
				low: 72,
				high: 86,
				confidence: "medium",
				criteria: "Speech naturalness",
				reasoning: "Publisher evidence and category comparison",
				sources: ["https://example.com/model"],
				updated_at: "2026-11-16",
			},
		},
	},
};

describe("specialist score coverage before media publication", () => {
	test("counts supported model categories while ignoring language models and support files", () => {
		expect(
			qualityCoverage([
				voice,
				{ ...voice, id: "file", type: "File" },
				{ ...voice, id: "chat", type: "Llm" },
			]),
		).toEqual({ categories: { Tts: { total: 1, rated: 1 } }, missing: [] });
	});
	test("rejects an unrelated category, unsupported score bounds and missing evidence", () => {
		for (const change of [
			{ category: "Stt" },
			{ score: 101 },
			{ low: 85 },
			{ high: 70 },
			{ sources: [] },
			{ sources: ["javascript:alert(1)"] },
			{ updated_at: "unknown" },
		]) {
			const altered = {
				...voice,
				model_evaluation: {
					evaluations: {
						quality_estimate: {
							...voice.model_evaluation.evaluations.quality_estimate,
							...change,
						},
					},
				},
			};
			expect(qualityCoverage([altered]).missing).toEqual(["voice"]);
		}
	});
	test("does not mistake a zero score for a missing evaluation", () => {
		const zero = {
			...voice,
			model_evaluation: {
				evaluations: {
					quality_estimate: {
						...voice.model_evaluation.evaluations.quality_estimate,
						score: 0,
						low: 0,
						high: 10,
					},
				},
			},
		};
		expect(qualityCoverage([zero]).categories.Tts.rated).toBe(1);
	});
});
