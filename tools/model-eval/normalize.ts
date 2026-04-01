import type { AAModel, GlobalMaxes, ModelClassification, TodoEntry } from "./types";

const MANUAL_FIELDS = ["creativity", "multilinguality", "openness", "safety"] as const;
const MIN_BENCHMARKS = 2;

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

function normalizeCoding(model: AAModel, maxes: GlobalMaxes): { value: number; missing: boolean } {
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

function normalizeReasoning(model: AAModel, maxes: GlobalMaxes): { value: number; missing: boolean } {
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

function normalizeFactuality(model: AAModel, maxes: GlobalMaxes): { value: number; missing: boolean } {
	const e = model.evaluations;
	const raw = [e.mmlu_pro, e.artificial_analysis_intelligence_index, e.gpqa];
	const max = [maxes.mmlu_pro, maxes.intelligence_index, maxes.gpqa];
	return safeNormalizedAvg(raw, max);
}

function normalizeFunctionCalling(model: AAModel, maxes: GlobalMaxes): { value: number; missing: boolean } {
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

	let effectivePrice = isLocal ? price / 3 : price;

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

	const reasoning = normalizeReasoning(model, maxes);
	if (reasoning.missing) missingFields.push("reasoning (partial benchmarks)");

	const factuality = normalizeFactuality(model, maxes);
	if (factuality.missing) missingFields.push("factuality (partial benchmarks)");

	const functionCalling = normalizeFunctionCalling(model, maxes);
	if (functionCalling.missing) missingFields.push("function_calling (partial benchmarks)");

	const speed = normalizeSpeed(model, maxes);
	if (speed === 0) missingFields.push("speed (no data)");

	const price = model.pricing.price_1m_blended_3_to_1;
	const cost = normalizeCost(price, allPrices, isLocal);
	if (cost === 0) missingFields.push("cost (no pricing data)");

	const classification: ModelClassification = {
		coding: round(coding.value),
		cost: round(cost),
		creativity: 0,
		factuality: round(factuality.value),
		function_calling: round(functionCalling.value),
		multilinguality: 0,
		openness: 0,
		reasoning: round(reasoning.value),
		safety: 0,
		speed: round(speed),
	};

	// For any computed metric that landed on 0, preserve the existing DB value if non-zero
	const COMPUTED_FIELDS = ["coding", "cost", "factuality", "function_calling", "reasoning", "speed"] as const;
	for (const field of COMPUTED_FIELDS) {
		if (classification[field] === 0) {
			const existing = existingClassification?.[field];
			if (existing != null && existing > 0) {
				classification[field] = round(existing);
			}
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
