import { expect, test } from "bun:test";
import { mergeBitParameters, upsertLlmModel } from "./db";
import type { AAModel, ModelClassification } from "./types";

const classification: ModelClassification = {
	coding: 0.5,
	cost: 0.7,
	creativity: 0.5,
	factuality: 0.5,
	function_calling: 0.5,
	multilinguality: 0.5,
	openness: 0.5,
	reasoning: 0.5,
	safety: 0.5,
	speed: 0.5,
};

test("updates estimate attribution together with ratings and preserves other parameters", () => {
	const evidence = {
		value: 0.5,
		confidence: "low" as const,
		reasoning: "Estimated from comparable models.",
		sources: ["https://example.com/model-card"],
		updated_at: "2026-10-27",
	};
	const original = {
		context_length: 128000,
		model_classification_estimates: { coding: evidence, reasoning: evidence },
		provider: {
			name: "openrouter",
			params: { tier: "PRO", model: "example/model" },
		},
	};
	const merged = mergeBitParameters(classification, original, {
		reasoning: evidence,
	});
	expect(merged.model_classification_estimates).toEqual({
		reasoning: evidence,
	});
	expect(merged.model_classification).toEqual(classification);
	expect(merged.context_length).toBe(128000);
	expect(merged.provider).toEqual({
		name: "openrouter",
		params: { tier: "PREMIUM", model: "example/model" },
	});
	expect(original.model_classification_estimates.coding).toEqual(evidence);
	expect(
		mergeBitParameters(classification, merged, {})
			.model_classification_estimates,
	).toEqual({});
});

test("AA refresh never upserts a catalog estimate record", async () => {
	const previous = process.env.DATABASE_URL;
	process.env.DATABASE_URL = "";
	try {
		await expect(
			upsertLlmModel({ slug: "flow-like-estimate-specialist" } as AAModel),
		).resolves.toBeUndefined();
	} finally {
		if (previous === undefined)
			Reflect.deleteProperty(process.env, "DATABASE_URL");
		else process.env.DATABASE_URL = previous;
	}
});
