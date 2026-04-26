import type {
	AAModel,
	GlobalMaxes,
	ModelClassification,
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
const MIN_BENCHMARKS = 2;

const OPENNESS_FALLBACKS = {
	"Open Weights (License Required for Commercial Use)": 27.77777777777778 / 100,
	"Open Weights (Permissive License)": 44.44444444444444 / 100,
	Proprietary: 11.11111111111111 / 100,
} as const;

function validValues(vals: (number | null | undefined)[]): number[] {
	return vals.filter((v): v is number => v != null && v > 0);
}

/** Average values that are each pre-normalized to [0,1] against their benchmark max. */
function safeNormalizedAvg(
	raw: (number | null | undefined)[],
	maxVals: number[],
): { value: number; missing: boolean } {
	const pairs: number[] = [];
	for (let i = 0; i < raw.length; i++) {
		const v = raw[i];
		const mx = maxVals[i];
		if (v != null && v > 0 && mx > 0) {
			pairs.push(Math.min(1.0, v / mx));
		}
	}
	if (pairs.length < MIN_BENCHMARKS) return { value: 0, missing: true };
	return {
		value: pairs.reduce((a, b) => a + b, 0) / pairs.length,
		missing: pairs.length < raw.length,
	};
}

function normalizeCoding(
	model: AAModel,
	maxes: GlobalMaxes,
): { value: number; missing: boolean } {
	const e = model.evaluations;
	const raw = [
		e.artificial_analysis_coding_index,
		e.livecodebench,
		e.scicode,
		e.terminalbench_hard,
	];
	const max = [
		maxes.coding_index,
		maxes.livecodebench,
		maxes.scicode,
		maxes.terminalbench_hard,
	];
	return safeNormalizedAvg(raw, max);
}

function normalizeCreativity(model: AAModel): number {
	const score = model.evaluations.writing_benchmark_mean_score;
	if (score == null || score <= 0) return 0;
	return clampUnit(score / 10);
}

function normalizeReasoning(
	model: AAModel,
	maxes: GlobalMaxes,
): { value: number; missing: boolean } {
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
): { value: number; missing: boolean } {
	const e = model.evaluations;
	const raw = [e.mmlu_pro, e.artificial_analysis_intelligence_index, e.gpqa];
	const max = [maxes.mmlu_pro, maxes.intelligence_index, maxes.gpqa];
	return safeNormalizedAvg(raw, max);
}

function normalizeFunctionCalling(
	model: AAModel,
	maxes: GlobalMaxes,
): { value: number; missing: boolean } {
	const e = model.evaluations;
	const raw = [e.tau2, e.ifbench, e.terminalbench_hard];
	const max = [maxes.tau2, maxes.ifbench, maxes.terminalbench_hard];
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

function normalizeMultilinguality(model: AAModel): number {
	const score =
		model.evaluations.artificial_analysis_multilingual_index_normalized;
	if (score == null || score <= 0) return 0;
	return clampUnit(score);
}

function normalizeOpenness(model: AAModel): number {
	const score = model.evaluations.artificial_analysis_openness_index;
	if (score != null && score > 0) {
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
	if (normalizedIndex != null && normalizedIndex > 0) {
		return {
			value: clampUnit(normalizedIndex),
			missing: false,
			partial: false,
		};
	}

	const components: number[] = [];
	if (
		e.artificial_analysis_omniscience_accuracy != null &&
		e.artificial_analysis_omniscience_accuracy > 0
	) {
		components.push(clampUnit(e.artificial_analysis_omniscience_accuracy));
	}

	if (
		e.artificial_analysis_omniscience_hallucination_rate != null &&
		e.artificial_analysis_omniscience_hallucination_rate >= 0
	) {
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
): { classification: ModelClassification; missingFields: string[] } {
	const missingFields: string[] = [];

	const coding = normalizeCoding(model, maxes);
	if (coding.missing) missingFields.push("coding (partial benchmarks)");

	const creativity = normalizeCreativity(model);
	if (creativity === 0) missingFields.push("creativity (no benchmark data)");

	const reasoning = normalizeReasoning(model, maxes);
	if (reasoning.missing) missingFields.push("reasoning (partial benchmarks)");

	const factuality = normalizeFactuality(model, maxes);
	if (factuality.missing) missingFields.push("factuality (partial benchmarks)");

	const functionCalling = normalizeFunctionCalling(model, maxes);
	if (functionCalling.missing)
		missingFields.push("function_calling (partial benchmarks)");

	const speed = normalizeSpeed(model, maxes);
	if (speed === 0) missingFields.push("speed (no data)");

	const price = model.pricing.price_1m_blended_3_to_1;
	const cost = normalizeCost(price, allPrices, isLocal);
	if (cost === 0) missingFields.push("cost (no pricing data)");

	const multilinguality = normalizeMultilinguality(model);
	if (multilinguality === 0)
		missingFields.push("multilinguality (no benchmark data)");

	const openness = normalizeOpenness(model);
	if (openness === 0) missingFields.push("openness (no benchmark data)");

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

	// For any computed metric that landed on 0, preserve the existing DB value if non-zero
	for (const field of COMPUTED_FIELDS) {
		if (classification[field] === 0) {
			const existing = existingClassification?.[field];
			if (existing != null && existing > 0) {
				classification[field] = round(existing);
			}
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

	return { classification, missingFields };
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
