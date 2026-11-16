import { type AAV5Model, AA_V5_VERSION } from "./aa-v5";

export type JsonRecord = Record<string, unknown>;
export interface ScorePair {
	slug: string;
	creator: string;
	previous: number;
	current: number;
	source: string;
}
export interface ScorePrior {
	score: number;
	low: number;
	high: number;
	source: string;
	basis: string;
}
export interface Calibration {
	method: "offset" | "affine" | "weighted_ratio";
	intercept: number | null;
	slope: number | null;
	weighted_ratio?: { neighbors: number; distance_floor: number };
	pairs: ScorePair[];
	validation: {
		leave_one_out_mae: number;
		leave_creator_out_mae: number;
		p90_absolute_error: number;
		offset_mae: number;
		affine_mae: number | null;
		weighted_ratio_mae: number | null;
	};
	previous_range: [number, number];
}

const WEIGHTED_RATIO_PARAMETERS = { neighbors: 7, distance_floor: 5 } as const;

export function record(value: unknown): JsonRecord {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as JsonRecord)
		: {};
}
export function score(value: unknown): value is number {
	return (
		typeof value === "number" &&
		Number.isFinite(value) &&
		value >= 0 &&
		value <= 100
	);
}
function mean(values: number[]): number {
	return values.reduce((a, b) => a + b, 0) / values.length;
}
function quantile(values: number[], fraction: number): number {
	const sorted = [...values].sort((a, b) => a - b);
	return sorted[
		Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)
	];
}
function clamp(value: number): number {
	return Math.max(0, Math.min(100, value));
}
function fit(pairs: ScorePair[], method: "offset" | "affine") {
	if (method === "offset") {
		return {
			slope: 1,
			intercept: quantile(
				pairs.map((p) => p.current - p.previous),
				0.5,
			),
		};
	}
	const x = mean(pairs.map((p) => p.previous));
	const y = mean(pairs.map((p) => p.current));
	const variance = pairs.reduce((sum, p) => sum + (p.previous - x) ** 2, 0);
	const slope =
		variance > 0
			? pairs.reduce((sum, p) => sum + (p.previous - x) * (p.current - y), 0) /
				variance
			: 1;
	if (slope <= 0 || slope > 2)
		throw new Error("Unstable legacy-to-v5 calibration slope");
	return { slope, intercept: y - slope * x };
}

function weightedRatio(
	pairs: ScorePair[],
	value: number,
	parameters: NonNullable<Calibration["weighted_ratio"]>,
): number {
	if (
		!Number.isInteger(parameters.neighbors) ||
		parameters.neighbors < 1 ||
		!Number.isFinite(parameters.distance_floor) ||
		parameters.distance_floor <= 0
	) {
		throw new Error("Invalid weighted-ratio calibration parameters");
	}
	const donors = pairs.filter((pair) => pair.previous > 0);
	if (donors.length < 4)
		throw new Error("Too few positive-score calibration donors");
	// The neighborhood and distance floor are fixed before validation, not tuned to its errors.
	const nearest = donors
		.sort(
			(a, b) =>
				Math.abs(a.previous - value) - Math.abs(b.previous - value) ||
				a.slug.localeCompare(b.slug),
		)
		.slice(0, parameters.neighbors);
	let totalWeight = 0;
	let weightedRatios = 0;
	for (const pair of nearest) {
		const weight =
			1 / (Math.abs(pair.previous - value) + parameters.distance_floor) ** 2;
		totalWeight += weight;
		weightedRatios += (weight * pair.current) / pair.previous;
	}
	return clamp((value * weightedRatios) / totalWeight);
}

export function predictCalibratedScore(
	calibration: Calibration,
	value: number,
): number {
	if (!score(value))
		throw new Error("Invalid score for calibration prediction");
	if (calibration.method === "weighted_ratio") {
		if (!calibration.weighted_ratio)
			throw new Error("Missing weighted-ratio calibration parameters");
		return weightedRatio(calibration.pairs, value, calibration.weighted_ratio);
	}
	if (calibration.intercept === null || calibration.slope === null)
		throw new Error("Missing linear calibration coefficients");
	return clamp(calibration.intercept + calibration.slope * value);
}
function errors(
	pairs: ScorePair[],
	method: Calibration["method"],
	byCreator = false,
): number[] {
	return pairs.map((target) => {
		const donors = pairs.filter((p) =>
			byCreator ? p.creator !== target.creator : p.slug !== target.slug,
		);
		if (donors.length < 4)
			throw new Error("Too few independent calibration donors");
		if (method === "weighted_ratio")
			return Math.abs(
				target.current -
					weightedRatio(donors, target.previous, WEIGHTED_RATIO_PARAMETERS),
			);
		const model = fit(donors, method);
		return Math.abs(
			target.current - clamp(model.intercept + model.slope * target.previous),
		);
	});
}

/** Validate on held-out models and creators before choosing a conversion. */
export function calibrateScores(pairs: ScorePair[]): Calibration {
	if (
		pairs.length < 8 ||
		new Set(pairs.map((p) => p.slug)).size !== pairs.length
	)
		throw new Error(
			"Calibration requires at least eight distinct matched models",
		);
	if (
		new Set(pairs.map((p) => p.creator)).size < 3 ||
		pairs.some((p) => !score(p.previous) || !score(p.current))
	)
		throw new Error(
			"Calibration needs valid measurements from at least three creators",
		);
	const previousRange: [number, number] = [
		Math.min(...pairs.map((p) => p.previous)),
		Math.max(...pairs.map((p) => p.previous)),
	];
	if (previousRange[1] - previousRange[0] < 15)
		throw new Error("Calibration score range is too narrow");
	const offsetErrors = errors(pairs, "offset");
	const offsetCreator = errors(pairs, "offset", true);
	let affineErrors: number[] | undefined;
	let affineCreator: number[] | undefined;
	try {
		affineErrors = errors(pairs, "affine");
		affineCreator = errors(pairs, "affine", true);
		fit(pairs, "affine");
	} catch {
		// An unstable fitted slope disqualifies the optional affine candidate.
		affineErrors = undefined;
		affineCreator = undefined;
	}
	// A fitted slope must improve both validation views enough to justify its extra parameter.
	let method: Calibration["method"] = "offset";
	let selected = offsetErrors;
	let grouped = offsetCreator;
	if (
		affineErrors &&
		affineCreator &&
		mean(affineErrors) < mean(offsetErrors) * 0.9 &&
		mean(affineCreator) < mean(offsetCreator) * 0.9
	) {
		method = "affine";
		selected = affineErrors;
		grouped = affineCreator;
	}
	let ratioErrors: number[] | undefined;
	let ratioCreator: number[] | undefined;
	try {
		ratioErrors = errors(pairs, "weighted_ratio");
		ratioCreator = errors(pairs, "weighted_ratio", true);
	} catch {
		// Zero-valued legacy scores cannot supply a ratio. Other candidates remain available.
		ratioErrors = undefined;
		ratioCreator = undefined;
	}
	if (
		ratioErrors &&
		ratioCreator &&
		mean(ratioErrors) < mean(selected) * 0.9 &&
		mean(ratioCreator) < mean(grouped) * 0.9
	) {
		method = "weighted_ratio";
		selected = ratioErrors;
		grouped = ratioCreator;
	}
	return {
		method,
		...(method === "weighted_ratio"
			? {
					intercept: null,
					slope: null,
					weighted_ratio: WEIGHTED_RATIO_PARAMETERS,
				}
			: fit(pairs, method)),
		pairs,
		previous_range: previousRange,
		validation: {
			leave_one_out_mae: mean(selected),
			leave_creator_out_mae: mean(grouped),
			p90_absolute_error: Math.max(
				quantile(selected, 0.9),
				quantile(grouped, 0.9),
			),
			offset_mae: mean(offsetErrors),
			affine_mae: affineErrors ? mean(affineErrors) : null,
			weighted_ratio_mae: ratioErrors ? mean(ratioErrors) : null,
		},
	};
}

export function estimateV5(
	prior: ScorePrior,
	calibration: Calibration,
	verifiedAt: string,
) {
	if (
		![prior.score, prior.low, prior.high].every(score) ||
		prior.low > prior.score ||
		prior.high < prior.score
	)
		throw new Error("Invalid legacy score interval");
	const predict = (value: number) => predictCalibratedScore(calibration, value);
	const distance = Math.max(
		0,
		calibration.previous_range[0] - prior.score,
		prior.score - calibration.previous_range[1],
	);
	// These are conservative uncertainty bounds, not a statistical confidence interval.
	const margin =
		Math.max(5, calibration.validation.p90_absolute_error) + distance * 0.5;
	const value = predict(prior.score);
	const predictions = [predict(prior.low), value, predict(prior.high)];
	const low = Math.floor(Math.min(...predictions) - margin);
	const high = Math.ceil(Math.max(...predictions) + margin);
	const description =
		calibration.method === "offset"
			? "a median score offset"
			: calibration.method === "affine"
				? "a fitted slope and offset"
				: `distance-weighted current/legacy score ratios from up to ${calibration.weighted_ratio?.neighbors} nearest positive-score donors, with weight 1/(score distance + ${calibration.weighted_ratio?.distance_floor})²`;
	return {
		score: Math.round(value * 10) / 10,
		low: clamp(low),
		high: clamp(high),
		confidence: "low" as const,
		version: AA_V5_VERSION,
		method: "paired-version-calibration" as const,
		reasoning: `Estimated from ${calibration.pairs.length} matched legacy/current model pairs using ${description}. Held-out mean absolute error is ${calibration.validation.leave_one_out_mae.toFixed(2)} points; holding out entire creators gives ${calibration.validation.leave_creator_out_mae.toFixed(2)} points. The legacy catalog mixed index vintages, so this is not an exact v4.3 conversion. ${prior.basis} Bounds include prior uncertainty and the larger 90th-percentile validation error${distance > 0 ? ", widened for extrapolation below or above the paired score range" : ""}. They are heuristic uncertainty bounds. This is a catalog estimate, not an Artificial Analysis measurement.`,
		sources: [
			...new Set([prior.source, ...calibration.pairs.map((p) => p.source)]),
		],
		updated_at: verifiedAt,
		prior,
		calibration: {
			method: calibration.method,
			intercept: calibration.intercept,
			slope: calibration.slope,
			...(calibration.weighted_ratio
				? { weighted_ratio: calibration.weighted_ratio }
				: {}),
			previous_range: calibration.previous_range,
			validation: calibration.validation,
			pairs: calibration.pairs,
		},
	};
}

export const V5_FIELD_MAP = {
	briefcaseElo: "aa_briefcase_elo",
	automationBenchPartialScore: "automationbench_aa",
	terminalBench40: "terminalbench_v4_0",
	frontierCode20: "frontiercode_v2_0",
	omniscience: "artificial_analysis_omniscience",
	gdpPdfAllPass: "gdp_pdf_all_pass",
	hleDiamond: "hle_diamond",
	critptAgenticHard: "critpt_agentic_hard",
	terminalBenchScience: "terminalbench_science_v0_1",
} as const;

export function legacyEvaluations(evaluations: JsonRecord): JsonRecord {
	return evaluations.legacy_evaluations &&
		typeof evaluations.legacy_evaluations === "object" &&
		!Array.isArray(evaluations.legacy_evaluations)
		? record(evaluations.legacy_evaluations)
		: evaluations;
}

export function migrateEvaluations(
	previous: JsonRecord,
	model: AAV5Model | undefined,
	estimate: ReturnType<typeof estimateV5> | undefined,
	verifiedAt: string,
) {
	const measured = score(model?.intelligenceIndex);
	const evaluations: JsonRecord = {};
	// Writing, multilingual and publisher evidence have their own methods and versions.
	for (const [key, value] of Object.entries(previous)) {
		if (
			key.startsWith("writing_benchmark_") ||
			key.startsWith("artificial_analysis_multilingual_") ||
			key === "artificial_analysis_openness_index" ||
			key === "reported_benchmarks"
		)
			evaluations[key] = value;
	}
	for (const [sourceKey, targetKey] of Object.entries(V5_FIELD_MAP)) {
		evaluations[targetKey] =
			model?.evaluations[sourceKey as keyof typeof V5_FIELD_MAP] ?? null;
	}
	return {
		...evaluations,
		artificial_analysis_intelligence_index: measured
			? model.intelligenceIndex
			: null,
		artificial_analysis_intelligence_index_version: AA_V5_VERSION,
		artificial_analysis_omniscience_accuracy:
			model?.omniscienceAccuracy ?? null,
		artificial_analysis_omniscience_hallucination_rate:
			model?.omniscienceHallucinationRate ?? null,
		benchmark_status: measured
			? "measured"
			: estimate
				? "estimated"
				: model
					? "not_evaluated"
					: "unmatched",
		benchmark_match_status: model ? "exact_model" : "unmatched",
		benchmark_source_url:
			model?.sourceUrl ?? "https://artificialanalysis.ai/models",
		benchmark_verified_at: model?.fetchedAt ?? verifiedAt,
		benchmark_reasoning_effort: model?.effort?.slug ?? null,
		benchmark_model_slug: model?.slug ?? null,
		benchmark_model_name: model?.name ?? null,
		benchmark_methodology_url:
			"https://artificialanalysis.ai/methodology/intelligence-benchmarking",
		...(!measured && estimate ? { intelligence_estimate: estimate } : {}),
		legacy_evaluations: legacyEvaluations(previous),
	};
}

const AA_CAPABILITIES = [
	"coding",
	"reasoning",
	"function_calling",
	"factuality",
	"safety",
] as const;
function fraction(value: unknown): value is number {
	return (
		typeof value === "number" &&
		Number.isFinite(value) &&
		value >= 0 &&
		value <= 1
	);
}
function completeMean(values: unknown[]): number | undefined {
	return values.every(fraction) ? mean(values) : undefined;
}

export function migrateClassification(
	parameters: JsonRecord,
	model: AAV5Model | undefined,
	verifiedAt: string,
): JsonRecord {
	const previous = record(parameters.model_classification);
	const values: Record<(typeof AA_CAPABILITIES)[number], number | undefined> = {
		coding: completeMean([
			model?.evaluations.terminalBench40,
			model?.evaluations.frontierCode20,
		]),
		reasoning: completeMean([
			model?.evaluations.hleDiamond,
			model?.evaluations.critptAgenticHard,
			model?.evaluations.terminalBenchScience,
		]),
		function_calling: fraction(model?.evaluations.automationBenchPartialScore)
			? model.evaluations.automationBenchPartialScore
			: undefined,
		factuality: completeMean([
			model?.omniscienceAccuracy,
			fraction(model?.omniscienceHallucinationRate)
				? 1 - model.omniscienceHallucinationRate
				: undefined,
		]),
		// AA's intelligence suite does not measure harmful-content safety.
		safety: undefined,
	};
	const classification = { ...previous };
	const estimates = Object.fromEntries(
		Object.entries(record(parameters.model_classification_estimates)).filter(
			([key]) =>
				!AA_CAPABILITIES.includes(key as (typeof AA_CAPABILITIES)[number]),
		),
	);
	const unavailable = new Set(
		Array.isArray(parameters.classification_unavailable_fields)
			? (parameters.classification_unavailable_fields as string[])
			: [],
	);
	for (const key of AA_CAPABILITIES) {
		if (values[key] === undefined) unavailable.add(key);
		else unavailable.delete(key);
		// Existing runtimes require numbers. The unavailable list distinguishes missing from a measured zero.
		classification[key] =
			values[key] === undefined ? 0 : Math.round(values[key] * 1000) / 1000;
	}
	const oldMethod = record(parameters.classification_method);
	return {
		...parameters,
		model_classification: classification,
		model_classification_estimates: estimates,
		classification_estimated_fields: (Array.isArray(
			parameters.classification_estimated_fields,
		)
			? (parameters.classification_estimated_fields as string[])
			: Object.keys(estimates)
		).filter(
			(key) =>
				!AA_CAPABILITIES.includes(key as (typeof AA_CAPABILITIES)[number]),
		),
		classification_unavailable_fields: [...unavailable],
		legacy_classification: parameters.legacy_classification ?? {
			values: previous,
			estimates: parameters.model_classification_estimates ?? null,
			method: parameters.classification_method ?? null,
		},
		classification_method: {
			...oldMethod,
			version: `aa-v${AA_V5_VERSION}`,
			coding:
				"Mean of Terminal-Bench 4.0 and FrontierCode-AA 2.0 raw task fractions; both results required.",
			reasoning:
				"Mean of HLE-Diamond, CritPt v2 Hard and Terminal-Bench Science 0.1.0 raw task fractions; all results required.",
			function_calling: "AutomationBench-AA raw task fraction.",
			factuality:
				"Mean of AA-Omniscience v1.1 accuracy and one minus hallucination rate; both results required.",
			safety:
				"Unavailable. The AA intelligence suite does not measure harmful-content safety.",
			missing:
				"Unavailable fields use numeric zero for runtime compatibility; zero is not a measurement when listed in classification_unavailable_fields.",
			other_fields:
				"Cost, speed, openness, creativity and multilinguality retain their separate existing sources and are not measurements from the v5 intelligence suite.",
			sources: model ? [model.sourceUrl] : [],
			verified_at: verifiedAt,
		},
		benchmark_notes:
			"AA v5.0.1 evaluates a weighted set of primarily English text and agent tasks. The benchmark reasoning setting may differ from the runtime default. Scores for local quantizations and provider presets describe the named underlying model. Calibrated intelligence estimates are labeled separately from measured results. Other capability ratings use their own sources.",
	};
}
