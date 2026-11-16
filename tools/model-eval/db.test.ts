import { expect, test } from "bun:test";
import {
	assertIntelligenceIndexVersionUpdate,
	mergeBitParameters,
	upsertLlmModel,
} from "./db";
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

test("rejects stale or unversioned feeds before replacing a current evaluation", () => {
	const current = {
		artificial_analysis_intelligence_index_version: "5.0.1",
		artificial_analysis_intelligence_index: 42.891,
		benchmark_source: "https://artificialanalysis.ai/models/qwen4-max",
	};
	for (const version of [
		undefined,
		null,
		"",
		"4.3.2",
		"5.0",
		"5.0.0",
		"invalid",
	]) {
		expect(() =>
			assertIntelligenceIndexVersionUpdate(
				current,
				{ artificial_analysis_intelligence_index_version: version },
				"qwen4-max",
			),
		).toThrow("Refusing to replace qwen4-max Intelligence Index v5.0.1");
	}
	expect(current.artificial_analysis_intelligence_index).toBe(42.891);
	expect(current.benchmark_source).toBe(
		"https://artificialanalysis.ai/models/qwen4-max",
	);
});

test("protects versioned missing scores and compares all version components", () => {
	const existing = {
		artificial_analysis_intelligence_index_version: "v5.10.2",
		artificial_analysis_intelligence_index: null,
	};
	expect(() =>
		assertIntelligenceIndexVersionUpdate(
			existing,
			{ artificial_analysis_intelligence_index_version: "5.9.9" },
			"missing-current-score",
		),
	).toThrow("Refusing to replace");
	for (const version of ["5.10.2", "v5.10.3", "5.11", "6.0", 6]) {
		expect(() =>
			assertIntelligenceIndexVersionUpdate(
				existing,
				{ artificial_analysis_intelligence_index_version: version },
				"missing-current-score",
			),
		).not.toThrow();
	}
});

test("allows new records, legacy refreshes, and upgrading to the current version", () => {
	for (const existing of [
		undefined,
		null,
		{},
		{ artificial_analysis_intelligence_index_version: "4.3.2" },
	]) {
		for (const incoming of [
			{},
			{ artificial_analysis_intelligence_index_version: "5.0.1" },
		]) {
			expect(() =>
				assertIntelligenceIndexVersionUpdate(existing, incoming, "legacy"),
			).not.toThrow();
		}
	}
	expect(() =>
		assertIntelligenceIndexVersionUpdate(
			{ artificial_analysis_intelligence_index_version: "6.0" },
			{ artificial_analysis_intelligence_index_version: "5.0.1" },
			"future-version",
		),
	).toThrow("Refusing to replace");
});
