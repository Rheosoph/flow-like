import { describe, expect, test } from "bun:test";
import { type AAV5Model, AA_V5_VERSION } from "./aa-v5";
import {
	type Calibration,
	type ScorePair,
	type ScorePrior,
	calibrateScores,
	estimateV5,
	legacyEvaluations,
	migrateClassification,
	migrateEvaluations,
	predictCalibratedScore,
	record,
} from "./aa-v5-scoring";

const verifiedAt = "2026-11-16T12:00:00.000Z";
const sourceUrl = "https://artificialanalysis.ai/models/example-xhigh";

function pairs(transform: (value: number) => number): ScorePair[] {
	return [10, 20, 30, 40, 50, 60, 70, 80].map((previous, index) => ({
		slug: `model-${index}`,
		creator: `creator-${index % 4}`,
		previous,
		current: transform(previous),
		source: `https://artificialanalysis.ai/models/model-${index}`,
	}));
}

function creatorShiftPairs(): ScorePair[] {
	const values: Array<[number, number, string]> = [
		[15, 10, "a"],
		[35, 10, "a"],
		[55, 10, "a"],
		[75, 10, "a"],
		[20, -10, "b"],
		[40, -10, "b"],
		[60, -10, "b"],
		[80, -10, "b"],
		[25, 0, "c"],
		[65, 0, "c"],
		[30, 0, "d"],
		[70, 0, "d"],
	];
	return values.map(([previous, delta, creator], index) => ({
		slug: `shift-${index}`,
		creator,
		previous,
		current: previous + delta,
		source: sourceUrl,
	}));
}

function curvedPairs(): ScorePair[] {
	return Array.from({ length: 24 }, (_, index) => {
		const previous = 5 + 3.5 * index;
		return {
			slug: `curve-${index}`,
			creator: `creator-${index % 4}`,
			previous,
			current: previous * (0.15 + 0.008 * previous),
			source: sourceUrl,
		};
	});
}

function prior(overrides: Partial<ScorePrior> = {}): ScorePrior {
	return {
		score: 40,
		low: 40,
		high: 40,
		source: "https://example.com/original-score",
		basis: "Prior catalog score for the same model.",
		...overrides,
	};
}

function model(overrides: Partial<AAV5Model> = {}): AAV5Model {
	return {
		id: "model-id",
		slug: "example-xhigh",
		name: "Example (xhigh)",
		release: { slug: "example", name: "Example" },
		effort: { slug: "xhigh" },
		isReasoning: true,
		releaseDate: "2026-11-01",
		creator: { slug: "example-lab", name: "Example Lab" },
		intelligenceIndex: 40,
		evaluations: {
			briefcaseElo: 1600,
			automationBenchPartialScore: 0.6,
			terminalBench40: 0.2,
			frontierCode20: 0.4,
			omniscience: -4,
			gdpPdfAllPass: 0.3,
			hleDiamond: 0.1,
			critptAgenticHard: 0.2,
			terminalBenchScience: 0.3,
		},
		omniscienceAccuracy: 0.5,
		omniscienceHallucinationRate: 0.2,
		version: AA_V5_VERSION,
		sourceUrl,
		fetchedAt: verifiedAt,
		...overrides,
	};
}

describe("calibrateScores", () => {
	test("chooses a constant offset when a slope adds no held-out accuracy", () => {
		const calibration = calibrateScores(pairs((previous) => previous - 5));
		expect(calibration.method).toBe("offset");
		expect(calibration.intercept).toBe(-5);
		expect(calibration.slope).toBe(1);
		expect(calibration.validation.leave_one_out_mae).toBe(0);
		expect(calibration.validation.leave_creator_out_mae).toBe(0);
	});

	test("chooses an affine fit only when both held-out views improve", () => {
		const calibration = calibrateScores(
			pairs((previous) => previous * 0.5 + 5),
		);
		expect(calibration.method).toBe("affine");
		expect(calibration.intercept).toBe(5);
		expect(calibration.slope).toBe(0.5);
		expect(calibration.validation.leave_one_out_mae).toBe(0);
		expect(calibration.validation.leave_creator_out_mae).toBe(0);
		expect(calibration.validation.offset_mae).toBeGreaterThan(0);
	});

	test("holds out every model from a creator and retains the larger tail error", () => {
		const calibration = calibrateScores(creatorShiftPairs());
		expect(calibration.method).toBe("offset");
		// Eight individual errors are 10. Holding out creator a instead gives four errors of 20.
		expect(calibration.validation.leave_one_out_mae).toBeCloseTo(80 / 12);
		expect(calibration.validation.leave_creator_out_mae).toBe(10);
		expect(calibration.validation.p90_absolute_error).toBe(20);
	});

	test("rejects a slope that improves model holdout but fails creator holdout", () => {
		const calibration = calibrateScores(
			pairs((previous) => previous * 0.5).map((pair, index) => ({
				...pair,
				creator: `creator-${Math.floor(index / 2)}`,
				current: pair.current + (index >= 6 ? 20 : 0),
			})),
		);
		expect(calibration.validation.affine_mae).toBeLessThan(
			calibration.validation.offset_mae * 0.9,
		);
		expect(calibration.method).toBe("offset");
		expect(calibration.validation.leave_creator_out_mae).toBe(6.25);
	});

	test("disqualifies an unstable slope without discarding a usable offset", () => {
		const calibration = calibrateScores(
			pairs((previous) => previous).map((pair, index) => {
				const previous = index < 5 ? 0 : pair.previous;
				return { ...pair, previous, current: 90 - previous };
			}),
		);
		expect(calibration.method).toBe("offset");
		expect(calibration.validation.affine_mae).toBeNull();
		expect(Number.isFinite(calibration.validation.leave_one_out_mae)).toBe(
			true,
		);
		expect(JSON.stringify(calibration)).not.toContain("Infinity");
	});

	test("selects a local ratio curve with independently held-out model and creator errors", () => {
		const calibration = calibrateScores(curvedPairs());
		expect(calibration.method).toBe("weighted_ratio");
		expect(calibration.intercept).toBeNull();
		expect(calibration.slope).toBeNull();
		expect(calibration.weighted_ratio).toEqual({
			neighbors: 7,
			distance_floor: 5,
		});
		expect(calibration.validation.leave_one_out_mae).toBeCloseTo(
			0.6568252229438483,
			10,
		);
		expect(calibration.validation.leave_creator_out_mae).toBeCloseTo(
			0.6344073360733984,
			10,
		);
		expect(calibration.validation.weighted_ratio_mae).toBe(
			calibration.validation.leave_one_out_mae,
		);
		expect(calibration.validation.p90_absolute_error).toBeCloseTo(
			1.4562626463068113,
			10,
		);
	});

	test("uses seven nearby positive-score donors and ignores zero denominators", () => {
		const calibration: Calibration = {
			...calibrateScores(pairs((previous) => previous * 0.5)),
			method: "weighted_ratio",
			intercept: null,
			slope: null,
			weighted_ratio: { neighbors: 7, distance_floor: 5 },
			pairs: pairs((previous) => (previous === 80 ? 80 : previous * 0.5)),
		};
		expect(predictCalibratedScore(calibration, 10)).toBeCloseTo(5, 12);
		const withZero = {
			...calibration,
			pairs: [
				...calibration.pairs,
				{
					slug: "zero",
					creator: "zero-creator",
					previous: 0,
					current: 100,
					source: sourceUrl,
				},
			],
		};
		expect(predictCalibratedScore(withZero, 10)).toBeCloseTo(5, 12);
		expect(predictCalibratedScore(withZero, 0)).toBe(0);
		expect(() =>
			predictCalibratedScore(
				{ ...calibration, weighted_ratio: { neighbors: 7, distance_floor: 0 } },
				10,
			),
		).toThrow("Invalid weighted-ratio");
	});

	test("rejects duplicate identities, insufficient diversity and unsupported ranges", () => {
		const valid = pairs((previous) => previous - 5);
		expect(() => calibrateScores(valid.slice(0, 7))).toThrow("eight distinct");
		expect(() =>
			calibrateScores(valid.map((pair) => ({ ...pair, slug: "same-model" }))),
		).toThrow("eight distinct");
		expect(() =>
			calibrateScores(
				valid.map((pair) => ({ ...pair, creator: "same-creator" })),
			),
		).toThrow("three creators");
		expect(() =>
			calibrateScores(
				valid.map((pair, index) => ({ ...pair, previous: 40 + index })),
			),
		).toThrow("range is too narrow");
		expect(() =>
			calibrateScores(valid.map((pair) => ({ ...pair, current: Number.NaN }))),
		).toThrow("valid measurements");
		expect(() =>
			calibrateScores(
				valid.map((pair, index) => ({
					...pair,
					creator: index < 5 ? "dominant" : `minor-${index}`,
				})),
			),
		).toThrow("independent calibration donors");
	});
});

describe("estimateV5", () => {
	test("stores reproducible ratio parameters and propagates the prior through the local curve", () => {
		const calibration = calibrateScores(curvedPairs());
		const estimate = estimateV5(
			prior({ score: 30, low: 20, high: 40 }),
			calibration,
			verifiedAt,
		);
		const restored: Calibration = JSON.parse(
			JSON.stringify(estimate.calibration),
		);
		expect(predictCalibratedScore(restored, 30)).toBe(
			predictCalibratedScore(calibration, 30),
		);
		expect(estimate.reasoning).toContain(
			"distance-weighted current/legacy score ratios",
		);
		expect(estimate.reasoning).toContain("7 nearest positive-score donors");
		expect(estimate.low).toBeLessThanOrEqual(
			predictCalibratedScore(calibration, 20) - 5,
		);
		expect(estimate.high).toBeGreaterThanOrEqual(
			predictCalibratedScore(calibration, 40) + 5,
		);
		expect(estimate.confidence).toBe("low");
	});

	test("propagates the input interval through the fitted slope and adds uncertainty", () => {
		const calibration = calibrateScores(
			pairs((previous) => previous * 0.5 + 5),
		);
		const estimate = estimateV5(
			prior({ low: 20, high: 60 }),
			calibration,
			verifiedAt,
		);
		expect(estimate).toMatchObject({
			score: 25,
			low: 10,
			high: 40,
			confidence: "low",
			version: AA_V5_VERSION,
		});
		const exactPrior = estimateV5(prior(), calibration, verifiedAt);
		expect(exactPrior).toMatchObject({ score: 25, low: 20, high: 30 });
		expect(estimate.reasoning).toContain("legacy catalog mixed index vintages");
		expect(estimate.reasoning).toContain("heuristic uncertainty bounds");
		expect(estimate.reasoning).toContain(
			"not an Artificial Analysis measurement",
		);
		expect(estimate.calibration.pairs).toEqual(calibration.pairs);
		expect(estimate.prior.low).toBe(20);
	});

	test("uses creator-held-out tail error and deduplicates source evidence", () => {
		const estimate = estimateV5(
			prior({ source: sourceUrl }),
			calibrateScores(creatorShiftPairs()),
			verifiedAt,
		);
		expect(estimate).toMatchObject({ score: 40, low: 20, high: 60 });
		expect(estimate.sources).toEqual([sourceUrl]);
	});

	test("widens bounds for extrapolation and keeps scores within the index scale", () => {
		const calibration = calibrateScores(
			pairs((previous) => previous * 0.5 + 5),
		);
		const estimate = estimateV5(
			prior({ score: 0, low: 0, high: 0 }),
			calibration,
			verifiedAt,
		);
		expect(estimate).toMatchObject({ score: 5, low: 0, high: 15 });
		expect(estimate.reasoning).toContain("widened for extrapolation");
		expect(() =>
			estimateV5(prior({ low: 41 }), calibration, verifiedAt),
		).toThrow("Invalid legacy score interval");
		expect(() =>
			estimateV5(prior({ high: 101 }), calibration, verifiedAt),
		).toThrow("Invalid legacy score interval");
	});
});

describe("migrateEvaluations", () => {
	test("keeps a measured zero authoritative over an estimate and retains exact effort", () => {
		const estimate = estimateV5(
			prior(),
			calibrateScores(pairs((previous) => previous - 5)),
			verifiedAt,
		);
		const migrated = migrateEvaluations(
			{},
			model({ intelligenceIndex: 0 }),
			estimate,
			verifiedAt,
		);
		expect(migrated.artificial_analysis_intelligence_index).toBe(0);
		expect(migrated.benchmark_status).toBe("measured");
		expect(migrated.benchmark_reasoning_effort).toBe("xhigh");
		expect(migrated.benchmark_source_url).toBe(sourceUrl);
		expect(Object.hasOwn(migrated, "intelligence_estimate")).toBe(false);
	});

	test("archives obsolete raw results and keeps separately sourced ratings", () => {
		const previous = {
			artificial_analysis_intelligence_index: 70,
			artificial_analysis_coding_index: 90,
			gpqa: 0.9,
			hle: 0.8,
			aa_briefcase_elo: 1800,
			writing_benchmark_version: "pairwise",
			artificial_analysis_multilingual_index: 70,
			artificial_analysis_openness_index: 44,
			reported_benchmarks: [{ name: "Publisher benchmark", score: 80 }],
		};
		const before = structuredClone(previous);
		const migrated = migrateEvaluations(
			previous,
			model(),
			undefined,
			verifiedAt,
		);
		expect(migrated.legacy_evaluations).toEqual(before);
		expect(previous).toEqual(before);
		for (const oldKey of ["artificial_analysis_coding_index", "gpqa", "hle"]) {
			expect(Object.hasOwn(migrated, oldKey)).toBe(false);
		}
		expect(record(migrated).aa_briefcase_elo).toBe(1600);
		expect(record(migrated).artificial_analysis_omniscience).toBe(-4);
		expect(record(migrated).writing_benchmark_version).toBe("pairwise");
		expect(record(migrated).artificial_analysis_multilingual_index).toBe(70);
		expect(record(migrated).reported_benchmarks).toEqual(
			previous.reported_benchmarks,
		);
	});

	test("treats absent and null current components as unavailable instead of reusing old values", () => {
		const migrated = migrateEvaluations(
			{ frontiercode_v2_0: 0.9, terminalbench_v4_0: 0.8 },
			model({
				intelligenceIndex: null,
				evaluations: { terminalBench40: 0, frontierCode20: null },
			}),
			undefined,
			verifiedAt,
		);
		expect(migrated.artificial_analysis_intelligence_index).toBeNull();
		expect(record(migrated).terminalbench_v4_0).toBe(0);
		expect(record(migrated).frontiercode_v2_0).toBeNull();
		expect(record(migrated).hle_diamond).toBeNull();
		expect(migrated.benchmark_status).toBe("not_evaluated");
		expect(
			migrateEvaluations({}, undefined, undefined, verifiedAt).benchmark_status,
		).toBe("unmatched");
	});

	test("keeps estimated intelligence separate from the measured score", () => {
		const estimate = estimateV5(
			prior(),
			calibrateScores(pairs((previous) => previous - 5)),
			verifiedAt,
		);
		const migrated = migrateEvaluations(
			{},
			model({ intelligenceIndex: null }),
			estimate,
			verifiedAt,
		);
		expect(migrated.artificial_analysis_intelligence_index).toBeNull();
		expect(migrated.benchmark_status).toBe("estimated");
		expect(migrated.intelligence_estimate).toEqual(estimate);
	});

	test("repeated migration preserves the first archive, including an empty archive", () => {
		for (const previous of [
			{},
			{ gpqa: 0.9, artificial_analysis_intelligence_index: 60 },
		]) {
			const first = migrateEvaluations(
				previous,
				model(),
				undefined,
				verifiedAt,
			);
			const second = migrateEvaluations(first, model(), undefined, verifiedAt);
			expect(second).toEqual(first);
			expect(legacyEvaluations(second)).toEqual(previous);
			expect(
				Object.hasOwn(second.legacy_evaluations, "legacy_evaluations"),
			).toBe(false);
		}
	});
});

describe("migrateClassification", () => {
	test("requires complete task groups and preserves measured zeroes", () => {
		const migrated = migrateClassification(
			{},
			model({
				evaluations: {
					terminalBench40: 0,
					frontierCode20: 0,
					hleDiamond: 0,
					critptAgenticHard: 0,
					terminalBenchScience: 0,
					automationBenchPartialScore: 0,
				},
				omniscienceAccuracy: 0,
				omniscienceHallucinationRate: 1,
			}),
			verifiedAt,
		);
		expect(migrated.model_classification).toMatchObject({
			coding: 0,
			reasoning: 0,
			function_calling: 0,
			factuality: 0,
			safety: 0,
		});
		expect(migrated.classification_unavailable_fields).toEqual(["safety"]);
	});

	test("marks incomplete groups unavailable and clears their old estimates", () => {
		const migrated = migrateClassification(
			{
				model_classification: { coding: 0.9, reasoning: 0.8, factuality: 0.7 },
				model_classification_estimates: {
					coding: { score: 0.9 },
					creativity: { score: 0.6 },
				},
				classification_estimated_fields: ["coding", "creativity"],
			},
			model({
				evaluations: {
					terminalBench40: 0.8,
					frontierCode20: null,
					critptAgenticHard: 0.6,
					terminalBenchScience: 0.4,
					automationBenchPartialScore: 0,
				},
				omniscienceAccuracy: null,
			}),
			verifiedAt,
		);
		expect(migrated.model_classification).toMatchObject({
			coding: 0,
			reasoning: 0,
			factuality: 0,
			function_calling: 0,
		});
		expect(migrated.classification_unavailable_fields).toEqual([
			"coding",
			"reasoning",
			"factuality",
			"safety",
		]);
		expect(migrated.model_classification_estimates).toEqual({
			creativity: { score: 0.6 },
		});
		expect(migrated.classification_estimated_fields).toEqual(["creativity"]);
	});

	test("updates capability fractions without changing runtime, provider or pricing settings", () => {
		const parameters = {
			provider: "OpenRouter",
			model: "example/provider-model",
			pricing: { input: 1.5, output: 4.5 },
			temperature: 0.7,
			reasoning: { effort: "low" },
			context_length: 128_000,
			model_classification: {
				cost: 0.7,
				speed: 0.8,
				openness: 0.2,
				creativity: 0.6,
				multilinguality: 0.5,
				safety: 0.9,
			},
			classification_method: { cost: "Stored price source" },
		};
		const before = structuredClone(parameters);
		const migrated = migrateClassification(parameters, model(), verifiedAt);
		for (const key of [
			"provider",
			"model",
			"pricing",
			"temperature",
			"reasoning",
			"context_length",
		] as const) {
			expect(migrated[key]).toEqual(parameters[key]);
		}
		expect(parameters).toEqual(before);
		expect(migrated.model_classification).toMatchObject({
			cost: 0.7,
			speed: 0.8,
			openness: 0.2,
			creativity: 0.6,
			multilinguality: 0.5,
			coding: 0.3,
			reasoning: 0.2,
			function_calling: 0.6,
			factuality: 0.65,
			safety: 0,
		});
		expect(migrateClassification(migrated, model(), verifiedAt)).toEqual(
			migrated,
		);
		expect(migrated.legacy_classification).toMatchObject({
			values: before.model_classification,
			method: before.classification_method,
		});
	});
});
