import type {
	AAModel,
	CreativityBenchmarkMatchType,
	CreativitySimilarityTier,
	WritingBenchmarkVersion,
} from "./types";

export const WRITING_BENCHMARK_READMES: Array<{
	url: string;
	version: WritingBenchmarkVersion;
	heading: string;
}> = [
	{
		url: "https://raw.githubusercontent.com/lechmazur/writing/main/README.md",
		version: "pairwise",
		heading: "### Leaderboard",
	},
	{
		url: "https://raw.githubusercontent.com/lechmazur/writing/main/v3/README.md",
		version: "v3",
		heading: "#### Full overall leaderboard",
	},
	{
		url: "https://raw.githubusercontent.com/lechmazur/writing/main/v2/README.md",
		version: "v2",
		heading: "### Overall LLM Means",
	},
];

export interface CreativityBenchmarkEntry {
	modelName: string;
	meanScore: number | null;
	comparisonScore: number | null;
	estimatedWinChance: number | null;
	normalizedScore: number;
	version: WritingBenchmarkVersion;
	sourceUrl: string;
}

export interface ResolvedCreativityBenchmark
	extends Omit<CreativityBenchmarkEntry, "modelName"> {
	creativityEstimate: number | null;
	matchType: CreativityBenchmarkMatchType;
	sourceModelName: string | null;
	sourceModelSlug: string | null;
	similarity: number | null;
	similarityTier: CreativitySimilarityTier | null;
}

interface CreativitySimilarityFeature {
	key: string;
	weight: number;
	getValue: (model: AAModel) => number | null;
}

// AA omits the effort suffix for this model; retain the writer configuration in attribution.
const WRITING_MODEL_CONFIGURATIONS: Record<string, string> = {
	"kimi-k3-1-pro": "kimi-k3-1-pro-high",
};

export function parseWritingBenchmarks(
	readmes: string[],
): Map<string, CreativityBenchmarkEntry> {
	const benchmarks = new Map<string, CreativityBenchmarkEntry>();
	for (const [index, source] of WRITING_BENCHMARK_READMES.entries()) {
		for (const entry of parseWritingBenchmarkTable(
			readmes[index] ?? "",
			source.heading,
			source.version,
			source.url,
		)) {
			const key = normalizeCreativityKey(entry.modelName);
			if (key && !benchmarks.has(key)) benchmarks.set(key, entry);
		}
	}
	return benchmarks;
}

export function parseWritingBenchmarkTable(
	readme: string,
	heading: string,
	version: WritingBenchmarkVersion,
	sourceUrl: string,
): CreativityBenchmarkEntry[] {
	const lines = readme.split(/\r?\n/);
	const start = lines.findIndex(
		(line) => line.trim().toLowerCase() === heading.toLowerCase(),
	);
	if (start === -1) return [];
	const rows: string[][] = [];
	for (const line of lines.slice(start + 1)) {
		if (/^#{1,6}\s/.test(line.trim())) break;
		if (line.trim().startsWith("|"))
			rows.push(
				line
					.trim()
					.replace(/^\|/, "")
					.replace(/\|$/, "")
					.split("|")
					.map((cell) => cell.trim()),
			);
	}
	if (rows.length < 3) return [];
	const headings = rows[0].map((cell) => cell.toLowerCase());
	const modelIndex = headings.indexOf("model");
	const comparisonIndex = headings.indexOf("comparison score");
	const chanceIndex = headings.indexOf("estimated win chance");
	const pairwise = comparisonIndex >= 0 && chanceIndex >= 0;
	if (version === "pairwise" && !pairwise) return [];
	return rows.slice(2).flatMap<CreativityBenchmarkEntry>((columns) => {
		const modelName = (columns[modelIndex >= 0 ? modelIndex : 1] ?? "")
			.replace(/\*\*/g, "")
			.replace(/[†‡§¶^※]+$/u, "")
			.trim();
		if (!modelName) return [];
		if (pairwise) {
			const rawComparison = columns[comparisonIndex];
			const rawChance = columns[chanceIndex];
			if (!rawComparison || !rawChance || !/^\d+(?:\.\d+)?%$/.test(rawChance))
				return [];
			const comparisonScore = Number(rawComparison);
			const estimatedWinChance = Number(rawChance.slice(0, -1)) / 100;
			if (
				!Number.isFinite(comparisonScore) ||
				estimatedWinChance < 0 ||
				estimatedWinChance > 1
			)
				return [];
			return [
				{
					modelName,
					meanScore: null,
					comparisonScore,
					estimatedWinChance,
					normalizedScore: estimatedWinChance,
					version: "pairwise" as const,
					sourceUrl,
				},
			];
		}
		const rawMean = columns[2];
		if (!rawMean) return [];
		const meanScore = Number(rawMean);
		if (!Number.isFinite(meanScore) || meanScore < 0 || meanScore > 10)
			return [];
		return [
			{
				modelName,
				meanScore,
				comparisonScore: null,
				estimatedWinChance: null,
				normalizedScore: meanScore / 10,
				version,
				sourceUrl,
			},
		];
	});
}

export function matchCreativityBenchmark(
	model: AAModel,
	benchmarks: Map<string, CreativityBenchmarkEntry>,
): { entry: CreativityBenchmarkEntry; matchType: "direct" | "alias" } | null {
	for (const candidate of [
		model.name,
		model.slug,
		WRITING_MODEL_CONFIGURATIONS[model.slug],
	]) {
		if (!candidate) continue;
		const entry = benchmarks.get(normalizeCreativityKey(candidate));
		if (entry) return { entry, matchType: "direct" };
	}
	const alias = CREATIVITY_BENCHMARK_ALIASES[model.slug];
	const entry = alias ? benchmarks.get(alias) : undefined;
	return entry ? { entry, matchType: "alias" } : null;
}

export function resolveCreativityBenchmarks(
	models: AAModel[],
	benchmarks: Map<string, CreativityBenchmarkEntry>,
): Map<string, ResolvedCreativityBenchmark> {
	const resolved = new Map<string, ResolvedCreativityBenchmark>();
	for (const model of models) {
		const match = matchCreativityBenchmark(model, benchmarks);
		if (!match) continue;
		const { modelName, ...measurement } = match.entry;
		resolved.set(model.slug, {
			...measurement,
			creativityEstimate: null,
			matchType: match.matchType,
			sourceModelName: modelName,
			sourceModelSlug: match.matchType === "direct" ? model.slug : null,
			similarity: null,
			similarityTier: null,
		});
	}
	if (resolved.size === 0) return resolved;
	const donorModels = models.filter((model) => resolved.has(model.slug));
	const featureMaxes = computeCreativitySimilarityFeatureMaxes(models);
	for (const model of models) {
		if (resolved.has(model.slug)) continue;
		const match = matchCreativityBySimilarity(
			model,
			donorModels,
			resolved,
			featureMaxes,
		);
		if (match) resolved.set(model.slug, match);
	}
	return resolved;
}

export function summarizeCreativityBenchmarks(
	benchmarks: Map<string, ResolvedCreativityBenchmark>,
): { directOrAlias: number; similarity: number } {
	let directOrAlias = 0;
	let similarity = 0;
	for (const benchmark of benchmarks.values()) {
		if (benchmark.matchType === "similarity") similarity++;
		else directOrAlias++;
	}
	return { directOrAlias, similarity };
}

function safeMax(values: (number | null | undefined)[]): number {
	const valid = values.filter(
		(value): value is number => value != null && value > 0,
	);
	return valid.length ? Math.max(...valid) : 1;
}

const CREATIVITY_BENCHMARK_ALIASES: Record<string, string> = {
	"claude-opus-4-6-adaptive": "claude-opus-4-6",
	"claude-sonnet-4-6-adaptive": "claude-sonnet-4-5",
	"gemini-3-1-flash-lite-preview": "gemini-2-5-flash",
	"gemini-3-1-pro-preview": "gemini-3-pro",
	"glm-5": "glm-4-6",
	"gpt-5-4-pro": "gpt-5-pro",
	"minimax-m2": "minimax-m2-1",
	"minimax-m2-5": "minimax-m2-1",
};

const CREATIVITY_NEUTRAL_SCORE = 0.5;
const CREATIVITY_SIMILARITY_MIN_FEATURES = 3;
const CREATIVITY_SIMILARITY_TIER_DAMPING: Record<
	CreativitySimilarityTier,
	number
> = {
	same_creator_category: 1,
	same_creator: 0.95,
	same_category: 0.85,
};
const CREATIVITY_SIMILARITY_FEATURES: CreativitySimilarityFeature[] = [
	{
		key: "intelligence_index",
		weight: 1.5,
		getValue: (model) =>
			model.evaluations.artificial_analysis_intelligence_index,
	},
	{
		key: "coding_index",
		weight: 1,
		getValue: (model) => model.evaluations.artificial_analysis_coding_index,
	},
	{
		key: "math_index",
		weight: 1,
		getValue: (model) => model.evaluations.artificial_analysis_math_index,
	},
	{
		key: "gpqa",
		weight: 1,
		getValue: (model) => model.evaluations.gpqa,
	},
	{
		key: "mmlu_pro",
		weight: 0.8,
		getValue: (model) => model.evaluations.mmlu_pro,
	},
	{
		key: "livecodebench",
		weight: 0.7,
		getValue: (model) => model.evaluations.livecodebench,
	},
	{
		key: "ifbench",
		weight: 0.7,
		getValue: (model) => model.evaluations.ifbench,
	},
	{
		key: "tau2",
		weight: 0.7,
		getValue: (model) => model.evaluations.tau2,
	},
	{
		key: "output_tokens_per_second",
		weight: 0.4,
		getValue: (model) => model.median_output_tokens_per_second,
	},
];

function computeCreativitySimilarityFeatureMaxes(
	models: AAModel[],
): Map<string, number> {
	const featureMaxes = new Map<string, number>();

	for (const feature of CREATIVITY_SIMILARITY_FEATURES) {
		featureMaxes.set(
			feature.key,
			safeMax(models.map((model) => feature.getValue(model))),
		);
	}

	return featureMaxes;
}

function matchCreativityBySimilarity(
	model: AAModel,
	donorModels: AAModel[],
	resolvedBenchmarks: Map<string, ResolvedCreativityBenchmark>,
	featureMaxes: Map<string, number>,
): ResolvedCreativityBenchmark | null {
	const creatorSlug = model.model_creator.slug;
	const category = model.open_source_categorization ?? null;
	const candidateGroups: Array<{
		tier: CreativitySimilarityTier;
		candidates: AAModel[];
	}> = [];

	if (category) {
		candidateGroups.push({
			tier: "same_creator_category",
			candidates: donorModels.filter(
				(candidate) =>
					candidate.model_creator.slug === creatorSlug &&
					candidate.open_source_categorization === category,
			),
		});
	}

	candidateGroups.push({
		tier: "same_creator",
		candidates: donorModels.filter(
			(candidate) => candidate.model_creator.slug === creatorSlug,
		),
	});

	if (category) {
		candidateGroups.push({
			tier: "same_category",
			candidates: donorModels.filter(
				(candidate) => candidate.open_source_categorization === category,
			),
		});
	}

	for (const group of candidateGroups) {
		const bestMatch = selectBestCreativitySimilarityMatch(
			model,
			group.candidates,
			featureMaxes,
		);
		if (!bestMatch) continue;

		const sourceBenchmark = resolvedBenchmarks.get(bestMatch.candidate.slug);
		if (!sourceBenchmark) continue;

		const sourceScore = sourceBenchmark.normalizedScore;
		const similarity = clampUnit(
			(1 - bestMatch.distance) * CREATIVITY_SIMILARITY_TIER_DAMPING[group.tier],
		);
		const estimatedScore =
			CREATIVITY_NEUTRAL_SCORE +
			similarity * (sourceScore - CREATIVITY_NEUTRAL_SCORE);

		return {
			meanScore: null,
			comparisonScore: null,
			estimatedWinChance: null,
			creativityEstimate: roundToPrecision(estimatedScore, 3),
			normalizedScore: roundToPrecision(estimatedScore, 3),
			sourceUrl: sourceBenchmark.sourceUrl,
			version: sourceBenchmark.version,
			matchType: "similarity",
			sourceModelName: sourceBenchmark.sourceModelName,
			sourceModelSlug: bestMatch.candidate.slug,
			similarity: roundToPrecision(similarity, 3),
			similarityTier: group.tier,
		};
	}

	return null;
}

function selectBestCreativitySimilarityMatch(
	target: AAModel,
	candidates: AAModel[],
	featureMaxes: Map<string, number>,
): { candidate: AAModel; distance: number } | null {
	let bestMatch: { candidate: AAModel; distance: number } | null = null;

	for (const candidate of candidates) {
		const distance = computeCreativitySimilarityDistance(
			target,
			candidate,
			featureMaxes,
		);
		if (distance == null) continue;

		if (!bestMatch || distance < bestMatch.distance) {
			bestMatch = { candidate, distance };
		}
	}

	return bestMatch;
}

function computeCreativitySimilarityDistance(
	target: AAModel,
	candidate: AAModel,
	featureMaxes: Map<string, number>,
): number | null {
	let weightedDistance = 0;
	let totalWeight = 0;
	let overlap = 0;

	for (const feature of CREATIVITY_SIMILARITY_FEATURES) {
		const targetValue = feature.getValue(target);
		const candidateValue = feature.getValue(candidate);
		const featureMax = featureMaxes.get(feature.key) ?? 1;

		if (
			targetValue == null ||
			candidateValue == null ||
			targetValue <= 0 ||
			candidateValue <= 0 ||
			featureMax <= 0
		) {
			continue;
		}

		weightedDistance +=
			Math.min(1, Math.abs(targetValue - candidateValue) / featureMax) *
			feature.weight;
		totalWeight += feature.weight;
		overlap += 1;
	}

	if (overlap < CREATIVITY_SIMILARITY_MIN_FEATURES || totalWeight === 0) {
		return null;
	}

	return weightedDistance / totalWeight;
}

export function normalizeCreativityKey(value: string): string {
	return value
		.normalize("NFKD")
		.replace(/[^\p{ASCII}]/gu, "")
		.toLowerCase()
		.replace(/&/g, " and ")
		.replace(/\(no reasoning\)/g, "")
		.replace(/\(high reasoning\)/g, " high ")
		.replace(/\(medium reasoning\)/g, "")
		.replace(/\(low reasoning\)/g, " low ")
		.replace(/\(medium\)/g, "")
		.replace(/thinking 16k/g, "")
		.replace(/\./g, "-")
		.replace(/[^a-z0-9]+/g, "-")
		.replace(/-+/g, "-")
		.replace(/^-|-$/g, "");
}

function clampUnit(value: number): number {
	return Math.max(0, Math.min(1, value));
}

function roundToPrecision(value: number, places: number): number {
	const factor = 10 ** places;
	return Math.round(value * factor) / factor;
}
