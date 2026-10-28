import type {
	AAModel,
	GlobalMaxes,
	ModelClassification,
	ModelClassificationEstimates,
	TodoEntry,
} from "./types";

const MANUAL_FIELDS: (keyof ModelClassification)[] = [];
const COMPUTED_FIELDS = [
	"creativity",
	"coding",
	"cost",
	"factuality",
	"function_calling",
	"multilinguality",
	"openness",
	"reasoning",
	"safety",
	"speed",
] as const;

interface NormalizedBenchmark {
	value: number;
	missing: boolean;
	observed: boolean;
}

const OPENNESS_FALLBACKS = {
	"Open Weights (License Required for Commercial Use)": 27.77777777777778 / 100,
	"Open Weights (Permissive License)": 44.44444444444444 / 100,
	Proprietary: 11.11111111111111 / 100,
} as const;

function isBenchmark(value: number | null | undefined): value is number {
	return value != null && Number.isFinite(value) && value >= 0;
}

/** Average values that are each pre-normalized to [0,1] against their benchmark max. */
function safeNormalizedAvg(
	raw: (number | null | undefined)[],
	maxVals: number[],
): NormalizedBenchmark {
	const pairs: number[] = [];
	for (let i = 0; i < raw.length; i++) {
		const v = raw[i];
		const mx = maxVals[i];
		if (isBenchmark(v) && Number.isFinite(mx) && mx > 0) {
			pairs.push(Math.min(1.0, v / mx));
		}
	}
	if (pairs.length === 0) return { value: 0, missing: true, observed: false };
	return {
		value: pairs.reduce((a, b) => a + b, 0) / pairs.length,
		missing: pairs.length < raw.length,
		observed: true,
	};
}

function terminalBenchmark(
	model: AAModel,
	maxes: GlobalMaxes,
): [number | null, number] {
	const e = model.evaluations;
	// Versions measure different task sets. Use one version and its own maximum.
	if (isBenchmark(e.terminalbench_v4_0)) {
		return [e.terminalbench_v4_0, maxes.terminalbench_v4_0];
	}
	if (isBenchmark(e.terminalbench_v2_1)) {
		return [e.terminalbench_v2_1, maxes.terminalbench_v2_1];
	}
	return [e.terminalbench_hard, maxes.terminalbench_hard];
}

function normalizeCoding(
	model: AAModel,
	maxes: GlobalMaxes,
): NormalizedBenchmark {
	const e = model.evaluations;
	const [terminal, terminalMax] = terminalBenchmark(model, maxes);
	const raw = [
		e.artificial_analysis_coding_index,
		e.livecodebench,
		e.scicode,
		terminal,
	];
	const max = [
		maxes.coding_index,
		maxes.livecodebench,
		maxes.scicode,
		terminalMax,
	];
	return safeNormalizedAvg(raw, max);
}

function normalizeCreativity(model: AAModel): number {
	const evaluations = model.evaluations;
	const score =
		evaluations.writing_benchmark_estimated_win_chance ??
		evaluations.writing_benchmark_creativity_estimate ??
		(evaluations.writing_benchmark_mean_score == null
			? null
			: evaluations.writing_benchmark_mean_score / 10);
	return score != null && Number.isFinite(score) ? clampUnit(score) : 0;
}

function normalizeReasoning(
	model: AAModel,
	maxes: GlobalMaxes,
): NormalizedBenchmark {
	const e = model.evaluations;
	const raw = [
		e.artificial_analysis_math_index,
		e.aime,
		e.aime_25,
		e.math_500,
		e.hle,
		e.gpqa,
		e.lcr,
	];
	const max = [
		maxes.math_index,
		maxes.aime,
		maxes.aime_25,
		maxes.math_500,
		maxes.hle,
		maxes.gpqa,
		maxes.lcr,
	];
	return safeNormalizedAvg(raw, max);
}

function normalizeFactuality(
	model: AAModel,
	maxes: GlobalMaxes,
): NormalizedBenchmark {
	const e = model.evaluations;
	const accuracy = e.artificial_analysis_omniscience_accuracy;
	const hallucination = e.artificial_analysis_omniscience_hallucination_rate;
	if (isBenchmark(accuracy) || isBenchmark(hallucination)) {
		// Knowledge accuracy and non-hallucination measure factual reliability.
		return safeNormalizedAvg(
			[accuracy, isBenchmark(hallucination) ? 1 - hallucination : null],
			[1, 1],
		);
	}
	const raw = [e.mmlu_pro, e.gpqa];
	const max = [maxes.mmlu_pro, maxes.gpqa];
	return safeNormalizedAvg(raw, max);
}

function normalizeFunctionCalling(
	model: AAModel,
	maxes: GlobalMaxes,
): NormalizedBenchmark {
	const e = model.evaluations;
	// AutomationBench directly evaluates work performed through application APIs.
	if (isBenchmark(e.automationbench_aa)) {
		return safeNormalizedAvg(
			[e.automationbench_aa],
			[maxes.automationbench_aa],
		);
	}
	const raw = [e.tau_banking, e.tau2, e.ifbench];
	const max = [maxes.tau_banking, maxes.tau2, maxes.ifbench];
	return safeNormalizedAvg(raw, max);
}

function normalizeSpeed(model: AAModel, maxes: GlobalMaxes): number {
	const speed = model.median_output_tokens_per_second;
	if (speed == null || speed <= 0) return 0;
	const maxSpeed = maxes.median_output_tokens_per_second;
	if (maxSpeed <= 0) return 0;

	// Log-scale: ln(speed)/ln(max) → spreads the range instead of Mercury crushing everything
	const normalized = Math.log(speed) / Math.log(maxSpeed);
	return Math.max(0.05, Math.min(1.0, normalized));
}

function normalizeCost(
	price: number | null,
	allPrices: number[],
	isLocal: boolean,
): number {
	if (price == null || price <= 0) return 0;

	const effectivePrice = isLocal ? price / 3 : price;

	const validPrices = allPrices.filter((p) => p > 0);
	if (validPrices.length === 0) return 0;

	const logPrices = validPrices.map((p) => Math.log(p));
	const minLog = Math.min(...logPrices);
	const maxLog = Math.max(...logPrices);

	if (maxLog === minLog) return 1.0;

	const logPrice = Math.log(effectivePrice);
	const normalized = 1.0 - (logPrice - minLog) / (maxLog - minLog);

	return Math.max(0.05, Math.min(1.0, normalized * 0.95 + 0.05));
}

/** Catalog token rates are micro-USD per million tokens; AA prices are USD. */
export function catalogBlendedTokenPrice(
	parameters: Record<string, unknown> | null | undefined,
): number | undefined {
	const pricing = parameters?.pricing;
	if (!pricing || typeof pricing !== "object" || Array.isArray(pricing)) {
		return undefined;
	}
	const fields = pricing as Record<string, unknown>;
	const input = fields.input_micro_usd_per_million_tokens;
	const output = fields.output_micro_usd_per_million_tokens;
	if (
		typeof input !== "number" ||
		!isBenchmark(input) ||
		typeof output !== "number" ||
		!isBenchmark(output)
	) {
		return undefined;
	}
	// A request fee needs a workload token count and cannot become a token rate.
	if (fields.request_micro_usd != null && fields.request_micro_usd !== 0) {
		return undefined;
	}
	return (input * 0.75 + output * 0.25) / 1_000_000;
}

function normalizeMultilinguality(model: AAModel): number {
	const score =
		model.evaluations.artificial_analysis_multilingual_index_normalized ??
		model.evaluations.artificial_analysis_multilingual_index;
	if (!isBenchmark(score)) return 0;
	return clampUnit(score);
}

function normalizeOpenness(model: AAModel): number {
	const score = model.evaluations.artificial_analysis_openness_index;
	if (isBenchmark(score)) {
		return clampUnit(score / 100);
	}

	const category = model.open_source_categorization;
	if (!category) return 0;

	return OPENNESS_FALLS_BACK_TO_SCORE(category);
}

function normalizeSafety(model: AAModel): {
	value: number;
	missing: boolean;
	partial: boolean;
} {
	const e = model.evaluations;
	const normalizedIndex = e.artificial_analysis_omniscience_index_normalized;
	if (isBenchmark(normalizedIndex)) {
		return {
			value: clampUnit(normalizedIndex),
			missing: false,
			partial: false,
		};
	}

	const components: number[] = [];
	if (isBenchmark(e.artificial_analysis_omniscience_accuracy)) {
		components.push(clampUnit(e.artificial_analysis_omniscience_accuracy));
	}

	if (isBenchmark(e.artificial_analysis_omniscience_hallucination_rate)) {
		components.push(
			clampUnit(1 - e.artificial_analysis_omniscience_hallucination_rate),
		);
	}

	if (components.length === 0) {
		return { value: 0, missing: true, partial: false };
	}

	return {
		value:
			components.reduce((sum, value) => sum + value, 0) / components.length,
		missing: false,
		partial: components.length < 2,
	};
}

export function computeClassification(
	model: AAModel,
	maxes: GlobalMaxes,
	allPrices: number[],
	isLocal: boolean,
	existingClassification?: ModelClassification | null,
	todoOverrides?: Partial<ModelClassification>,
	priceOverride?: number,
	existingEstimates?: ModelClassificationEstimates | null,
): {
	classification: ModelClassification;
	missingFields: string[];
	classificationEstimates: ModelClassificationEstimates;
} {
	const missingFields: string[] = [];

	const coding = normalizeCoding(model, maxes);
	if (coding.missing) missingFields.push("coding (partial benchmarks)");

	const creativity = normalizeCreativity(model);
	const creativityObserved =
		isBenchmark(model.evaluations.writing_benchmark_estimated_win_chance) ||
		isBenchmark(model.evaluations.writing_benchmark_creativity_estimate) ||
		isBenchmark(model.evaluations.writing_benchmark_mean_score);
	if (!creativityObserved) missingFields.push("creativity (no benchmark data)");

	const reasoning = normalizeReasoning(model, maxes);
	if (reasoning.missing) missingFields.push("reasoning (partial benchmarks)");

	const factuality = normalizeFactuality(model, maxes);
	if (factuality.missing) missingFields.push("factuality (partial benchmarks)");

	const functionCalling = normalizeFunctionCalling(model, maxes);
	if (functionCalling.missing)
		missingFields.push("function_calling (partial benchmarks)");

	const speed = normalizeSpeed(model, maxes);
	if (speed === 0) missingFields.push("speed (no data)");

	const hasPriceOverride = isBenchmark(priceOverride);
	const price = hasPriceOverride
		? priceOverride
		: model.pricing.price_1m_blended_3_to_1;
	// Zero from the AA feed can mean unavailable pricing. A validated catalog
	// price of zero explicitly means free token usage.
	const cost =
		hasPriceOverride && price === 0
			? 1
			: normalizeCost(price, allPrices, isLocal);
	if (cost === 0) missingFields.push("cost (no pricing data)");

	const multilinguality = normalizeMultilinguality(model);
	const multilingualityObserved = isBenchmark(
		model.evaluations.artificial_analysis_multilingual_index_normalized ??
			model.evaluations.artificial_analysis_multilingual_index,
	);
	if (!multilingualityObserved)
		missingFields.push("multilinguality (no benchmark data)");

	const openness = normalizeOpenness(model);
	const opennessObserved =
		isBenchmark(model.evaluations.artificial_analysis_openness_index) ||
		Boolean(model.open_source_categorization);
	if (!opennessObserved) missingFields.push("openness (no benchmark data)");

	const safety = normalizeSafety(model);
	if (safety.missing) {
		missingFields.push("safety (no benchmark data)");
	} else if (safety.partial) {
		missingFields.push("safety (partial benchmark data)");
	}

	const classification: ModelClassification = {
		coding: round(coding.value),
		cost: round(cost),
		creativity: round(creativity),
		factuality: round(factuality.value),
		function_calling: round(functionCalling.value),
		multilinguality: round(multilinguality),
		openness: round(openness),
		reasoning: round(reasoning.value),
		safety: round(safety.value),
		speed: round(speed),
	};

	const observedBenchmarks = new Set<keyof ModelClassification>();
	for (const [field, result] of [
		["coding", coding],
		["reasoning", reasoning],
		["factuality", factuality],
		["function_calling", functionCalling],
	] as const) {
		if (result.observed) observedBenchmarks.add(field);
	}
	if (multilingualityObserved) observedBenchmarks.add("multilinguality");
	if (opennessObserved) observedBenchmarks.add("openness");
	if (!safety.missing) observedBenchmarks.add("safety");
	if (creativityObserved) observedBenchmarks.add("creativity");
	if (cost > 0) observedBenchmarks.add("cost");
	if (speed > 0) observedBenchmarks.add("speed");

	const measuredFields = new Set(observedBenchmarks);
	if (
		model.evaluations.writing_benchmark_match_type === "alias" ||
		model.evaluations.writing_benchmark_match_type === "similarity" ||
		(!isBenchmark(model.evaluations.writing_benchmark_estimated_win_chance) &&
			!isBenchmark(model.evaluations.writing_benchmark_mean_score))
	) {
		measuredFields.delete("creativity");
	}
	if (!isBenchmark(model.evaluations.artificial_analysis_openness_index)) {
		measuredFields.delete("openness");
	}

	// Keep prior ratings when data is unavailable, but retain measured zeros.
	for (const field of COMPUTED_FIELDS) {
		if (classification[field] === 0 && !observedBenchmarks.has(field)) {
			const existing = existingClassification?.[field];
			if (existing != null && existing > 0) {
				classification[field] = round(existing);
			}
		}
	}

	const classificationEstimates: ModelClassificationEstimates = {};
	for (const field of COMPUTED_FIELDS) {
		const estimate = existingEstimates?.[field];
		const existing = existingClassification?.[field];
		if (
			!measuredFields.has(field) &&
			estimate &&
			isBenchmark(existing) &&
			existing <= 1 &&
			estimate.value === existing
		) {
			// A generic similarity or license-category fallback must not erase a
			// researched estimate and attribute its value to a measured benchmark.
			classification[field] = round(existing);
			classificationEstimates[field] = {
				...estimate,
				value: classification[field],
			};
		}
	}

	for (const field of [
		"creativity",
		"multilinguality",
		"openness",
		"safety",
	] as const) {
		const override = todoOverrides?.[field];
		if (override != null && override > 0) {
			classification[field] = round(override);
		}
	}

	// Preserve existing DB values for manual fields, then apply todo overrides on top
	for (const field of MANUAL_FIELDS) {
		const existing = existingClassification?.[field];
		const override = todoOverrides?.[field];

		if (override != null && override > 0) {
			classification[field] = round(override);
		} else if (existing != null && existing > 0) {
			classification[field] = round(existing);
		} else {
			missingFields.push(field);
		}
	}

	for (const field of COMPUTED_FIELDS) {
		if (classificationEstimates[field]?.value !== classification[field]) {
			delete classificationEstimates[field];
		}
	}

	return { classification, missingFields, classificationEstimates };
}

function round(v: number): number {
	return Math.round(v * 100) / 100;
}

function clampUnit(v: number): number {
	return Math.max(0, Math.min(1, v));
}

function OPENNESS_FALLS_BACK_TO_SCORE(
	category: keyof typeof OPENNESS_FALLBACKS,
): number {
	return OPENNESS_FALLBACKS[category];
}

export function buildTodoList(
	entries: { slug: string; name: string; missing: string[] }[],
): TodoEntry[] {
	return entries
		.filter((e) => e.missing.length > 0)
		.map((e) => ({
			modelSlug: e.slug,
			modelName: e.name,
			missingFields: e.missing,
		}));
}
