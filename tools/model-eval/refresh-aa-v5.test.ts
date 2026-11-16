import { describe, expect, test } from "bun:test";
import { type AAV5Catalog, type AAV5Model, AA_V5_VERSION } from "./aa-v5";
import { record } from "./aa-v5-scoring";
import {
	type LanguageBit,
	assertBitUnchanged,
	calibrationPairs,
	evaluationSlug,
	planRefresh,
	refreshEnglishMetadata,
	scorePrior,
} from "./refresh-aa-v5";

const verifiedAt = "2026-11-16T12:00:00.000Z";

function fixture() {
	const bits: LanguageBit[] = [];
	const models: AAV5Model[] = [];
	for (let index = 0; index < 8; index++) {
		const slug = `model-${index}`;
		const name = `Model ${index} (Max)`;
		const creator = `creator-${index % 4}`;
		bits.push({
			id: `bit-${index}`,
			type: "Llm",
			model_slug: slug,
			meta: { en: { name, description: "A model for workflows." } },
			parameters: {
				provider: { model_id: `provider/${slug}`, params: { tier: "PRO" } },
				pricing: { input_micro_usd_per_million_tokens: 1000000 },
				context_length: 128000,
				model_classification: {
					coding: 0.9,
					reasoning: 0.9,
					speed: 0.6,
					cost: 0.7,
				},
			},
			model_evaluation: {
				slug,
				name,
				release_date: "2026-09-01",
				creator_slug: creator,
				creator_name: creator,
				pricing: { price_1m_input_tokens: 1 },
				evaluations: {
					artificial_analysis_intelligence_index: 20 + index * 4,
					gpqa: 0.8,
				},
			},
		});
		models.push({
			id: `aa-${index}`,
			slug,
			name,
			release: { slug, name },
			effort: { slug: "max" },
			isReasoning: true,
			releaseDate: "2026-09-01",
			creator: { slug: creator, name: creator },
			intelligenceIndex: 10 + index * 4,
			evaluations: {
				terminalBench40: 0.4,
				frontierCode20: 0.2,
				hleDiamond: 0.3,
				critptAgenticHard: 0.6,
				terminalBenchScience: 0.3,
				automationBenchPartialScore: 0,
			},
			omniscienceAccuracy: 0.4,
			omniscienceHallucinationRate: 0.2,
			version: AA_V5_VERSION,
			sourceUrl: `https://artificialanalysis.ai/models/${slug}`,
			fetchedAt: verifiedAt,
		});
	}
	const source: AAV5Catalog = {
		version: AA_V5_VERSION,
		models,
		pages: [
			{
				sourceUrl: "https://artificialanalysis.ai/models",
				fetchedAt: verifiedAt,
				status: 200,
				modelCount: models.length,
			},
		],
		missingSlugs: [],
	};
	return { bits, source };
}

function specialBit(id: string, slug: string, name: string): LanguageBit {
	return {
		id,
		type: "Vlm",
		model_slug: slug,
		meta: { en: { name, description: "A local model for workflows." } },
		parameters: {
			provider: { provider_name: "Local" },
			model_classification: { coding: 0.9, cost: 0.8 },
		},
		model_evaluation: {
			slug,
			name,
			evaluations: { artificial_analysis_intelligence_index: 33.9 },
		},
	};
}

describe("AA refresh planning", () => {
	test("refreshes benchmark copy from current metadata while preserving unrelated fields", () => {
		const metadata = {
			name: "Example",
			description: "Short card description.",
			long_description: "Keep this runtime configuration note.",
			tags: ["custom"],
			created_at: { secs_since_epoch: 123, nanos_since_epoch: 456 },
		};
		const evaluations = {
			artificial_analysis_intelligence_index: 42,
			artificial_analysis_intelligence_index_version: AA_V5_VERSION,
			benchmark_status: "measured",
			benchmark_source_url: "https://artificialanalysis.ai/models/example",
			benchmark_reasoning_effort: "max",
		};
		const updated = refreshEnglishMetadata(metadata, evaluations);
		expect(updated).toMatchObject({
			name: metadata.name,
			description: metadata.description,
			tags: metadata.tags,
			created_at: metadata.created_at,
		});
		expect(updated?.long_description).toContain(metadata.long_description);
		expect(updated?.long_description).toContain(AA_V5_VERSION);
		expect(refreshEnglishMetadata(updated, evaluations)).toBeUndefined();
		const concurrent = {
			...metadata,
			long_description: "The owner just changed this runtime note.",
			tags: ["new-tag"],
		};
		const refreshed = refreshEnglishMetadata(concurrent, evaluations);
		expect(refreshed?.tags).toEqual(["new-tag"]);
		expect(refreshed?.long_description).toContain(concurrent.long_description);
		expect(metadata.long_description).toBe(
			"Keep this runtime configuration note.",
		);
		for (const invalid of [
			null,
			{},
			{ ...metadata, description: 42 },
			{ ...metadata, long_description: [] },
		]) {
			expect(() => refreshEnglishMetadata(invalid, evaluations)).toThrow(
				"English model metadata",
			);
		}
	});

	test("calibration uses exact variants once and excludes current, unmatched and mismatched priors", () => {
		const { bits, source } = fixture();
		const models = new Map(source.models.map((model) => [model.slug, model]));
		const base = bits[0];
		const withEvaluations = (evaluations: Record<string, unknown>) => ({
			...base,
			model_evaluation: { ...base.model_evaluation, evaluations },
		});
		for (const version of ["5.0.1", "v5.0.1", "6.0", "V6.1"]) {
			expect(
				calibrationPairs(
					[
						withEvaluations({
							artificial_analysis_intelligence_index: 20,
							artificial_analysis_intelligence_index_version: version,
						}),
					],
					models,
				),
			).toEqual([]);
		}
		for (const field of ["hle_diamond", "critpt_agentic_hard"]) {
			expect(
				calibrationPairs(
					[
						withEvaluations({
							artificial_analysis_intelligence_index: 20,
							[field]: 0,
						}),
					],
					models,
				),
			).toEqual([]);
		}
		expect(
			calibrationPairs([{ ...base, model_slug: "unmatched" }], models),
		).toEqual([]);
		expect(
			calibrationPairs(
				[
					{
						...base,
						model_evaluation: {
							...base.model_evaluation,
							name: "Model 0 (High)",
						},
					},
				],
				models,
			),
		).toEqual([]);
		expect(
			calibrationPairs(
				[
					withEvaluations({
						artificial_analysis_intelligence_index: 20,
						benchmark_reasoning_effort: "high",
					}),
				],
				models,
			),
		).toEqual([]);
		models.set("qwen4-max", { ...source.models[0], slug: "qwen4-max" });
		expect(
			calibrationPairs([{ ...base, model_slug: "qwen4-max" }], models),
		).toEqual([]);
		expect(
			calibrationPairs(
				[...bits, { ...base, id: "same-model-second-bit" }],
				models,
			),
		).toHaveLength(8);
		expect(() =>
			calibrationPairs(
				[base, withEvaluations({ artificial_analysis_intelligence_index: 21 })],
				models,
			),
		).toThrow("Conflicting legacy results");
	});

	test("corrects the Qwen artifact and uses only its exact historical prior", () => {
		const bit = {
			...specialBit(
				"ngbc8xvkrv5mtvcjq81w9n6v",
				"qwen3-4b-instruct-reasoning",
				"Qwen3 4B",
			),
			file_name: "Qwen3-4B-Instruct-2507-UD-Q5_K_XL.gguf",
		};
		expect(evaluationSlug(bit)).toBe("qwen3-4b-2507-instruct");
		expect(scorePrior(bit, [])).toBeUndefined();
		const historical = [
			{
				slug: "qwen3-4b-2507-instruct",
				name: "Qwen3 4B 2507 Instruct",
				evaluations: { artificial_analysis_intelligence_index: 6.7 },
			},
		];
		expect(scorePrior(bit, historical)?.score).toBe(6.7);
		expect(
			scorePrior(bit, [
				{
					...historical[0],
					evaluations: {
						...historical[0].evaluations,
						artificial_analysis_intelligence_index_version: "v5.0.1",
					},
				},
			]),
		).toBeUndefined();
		expect(() =>
			evaluationSlug({ ...bit, file_name: "different.gguf" }),
		).toThrow("artifact identity changed");
		const { bits, source } = fixture();
		const plan = planRefresh([...bits, bit], source, historical).plans[
			bits.length
		];
		expect(plan.slug).toBe("qwen3-4b-2507-instruct");
		expect(plan.model.slug).toBe(plan.slug);
		expect(
			record(record(plan.model.evaluations).intelligence_estimate).prior,
		).toMatchObject({ score: 6.7 });
	});

	test("keeps the uncensored artifact separate from the measured base model", () => {
		const { bits, source } = fixture();
		const bit = specialBit(
			"m3mnsg6v81uvikfmr6nssurl",
			"qwen3-8-27b",
			"Qwen3.8 27B Uncensored",
		);
		source.models.push({
			...source.models[0],
			slug: "qwen3-8-27b",
			intelligenceIndex: 20.1,
		});
		const plan = planRefresh([...bits, bit], source).plans[bits.length];
		expect(plan.slug).toBe("flow-like-estimate-qwen3-8-27b-uncensored");
		expect(plan.model.slug).toBe(plan.slug);
		const evaluation = record(plan.model.evaluations);
		expect(evaluation.artificial_analysis_intelligence_index).toBeNull();
		expect(evaluation.benchmark_status).toBe("estimated");
		expect(
			record(record(evaluation.intelligence_estimate).prior),
		).toMatchObject({ score: 33.9, low: 26.9, high: 40.9 });
		expect(plan.parameters.classification_unavailable_fields).toContain(
			"coding",
		);
	});

	test("archives old measurements while preserving runtime configuration and measured zero", () => {
		const { bits, source } = fixture();
		const before = JSON.stringify({ bits, source });
		const { plans } = planRefresh(bits, source);
		expect(JSON.stringify({ bits, source })).toBe(before);
		for (const plan of plans) {
			const evaluation = record(plan.model.evaluations);
			expect(evaluation.artificial_analysis_intelligence_index_version).toBe(
				AA_V5_VERSION,
			);
			expect(evaluation.gpqa).toBeUndefined();
			expect(record(evaluation.legacy_evaluations).gpqa).toBe(0.8);
			expect(evaluation.benchmark_reasoning_effort).toBe("max");
			expect(plan.parameters.provider).toEqual(plan.bit.parameters?.provider);
			expect(plan.parameters.pricing).toEqual(plan.bit.parameters?.pricing);
			expect(plan.parameters.context_length).toBe(128000);
			expect(plan.model.pricing).toEqual(plan.bit.model_evaluation?.pricing);
			expect(record(plan.parameters.model_classification)).toMatchObject({
				coding: 0.3,
				reasoning: 0.4,
				function_calling: 0,
				speed: 0.6,
				cost: 0.7,
			});
			expect(plan.parameters.classification_unavailable_fields).not.toContain(
				"function_calling",
			);
			expect(plan.parameters.classification_unavailable_fields).toContain(
				"safety",
			);
		}
		const migrated = plans.map((plan) => ({
			...plan.bit,
			model_slug: plan.slug,
			model_evaluation: plan.model,
			parameters: plan.parameters,
		}));
		expect(
			planRefresh(migrated, source).plans.map((plan) => plan.model),
		).toEqual(plans.map((plan) => plan.model));
	});

	test("rejects conflicting source and shared model records before writes", () => {
		const { bits, source } = fixture();
		expect(() => planRefresh([...bits, bits[0]], source)).toThrow(
			"Duplicate catalog Bit IDs",
		);
		expect(() =>
			planRefresh(bits, {
				...source,
				models: [
					...source.models,
					{ ...source.models[0], intelligenceIndex: 99 },
				],
			}),
		).toThrow("Conflicting source models");
		expect(() =>
			planRefresh(bits, {
				...source,
				models: [
					{ ...source.models[0], version: "4.3.2" } as unknown as AAV5Model,
					...source.models.slice(1),
				],
			}),
		).toThrow("Unexpected source index version");
		const conflictingBit = {
			...bits[0],
			id: "same-model-other-price",
			model_evaluation: {
				...bits[0].model_evaluation,
				pricing: { price_1m_input_tokens: 99 },
			},
		};
		expect(() => planRefresh([...bits, conflictingBit], source)).toThrow(
			"Conflicting planned evaluations",
		);
	});

	test("checks concurrent changes without treating JSON key order as a change", () => {
		const { bits } = fixture();
		const bit = bits[0];
		const reordered = {
			...bit,
			parameters: Object.fromEntries(
				Object.entries(record(bit.parameters)).reverse(),
			),
		};
		expect(() => assertBitUnchanged(bit, reordered)).not.toThrow();
		const changedEvaluation = {
			...bit,
			model_evaluation: { ...bit.model_evaluation, name: "updated externally" },
		};
		expect(() => assertBitUnchanged(bit, changedEvaluation)).toThrow(
			"changed after planning",
		);
		expect(() =>
			assertBitUnchanged(bit, changedEvaluation, false),
		).not.toThrow();
		for (const changes of [
			{ id: "other" },
			{ model_slug: "other" },
			{ file_name: "other.gguf" },
			{ parameters: { ...bit.parameters, context_length: 1 } },
		]) {
			expect(() =>
				assertBitUnchanged(bit, { ...bit, ...changes }, false),
			).toThrow("changed after planning");
		}
	});
});
