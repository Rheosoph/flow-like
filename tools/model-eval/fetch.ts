import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import type {
	AAApiResponse,
	AAEvaluations,
	AAModel,
	AAOpenSourceCategorization,
	CreativityBenchmarkMatchType,
	CreativitySimilarityTier,
	GlobalMaxes,
	ResultsFile,
	WritingBenchmarkVersion,
} from "./types";

const API_URL = "https://artificialanalysis.ai/api/v2/data/llms/models";
const MODELS_PAGE_URL = "https://artificialanalysis.ai/models";
const MULTILINGUAL_URL = "https://artificialanalysis.ai/models/multilingual";
const OMNISCIENCE_URL = "https://artificialanalysis.ai/evaluations/omniscience";
const WRITING_BENCHMARK_READMES: Array<{
	url: string;
	version: WritingBenchmarkVersion;
	heading: string;
}> = [
	{
		url: "https://raw.githubusercontent.com/lechmazur/writing/main/README.md",
		version: "v4",
		heading: "#### Full overall leaderboard",
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

const MULTILINGUAL_DATASET = "Multilingual Index Across Languages (Normalized)";
const OMNISCIENCE_INDEX_DATASET =
	"AA-Omniscience Index Across Domains (Normalized)";
const OMNISCIENCE_ACCURACY_DATASET = "AA-Omniscience Accuracy";
const OMNISCIENCE_HALLUCINATION_DATASET = "AA-Omniscience Hallucination Rate";

type JsonObject = Record<string, unknown>;

interface CreativityBenchmarkEntry {
	modelName: string;
	score: number;
	version: WritingBenchmarkVersion;
}

interface CreativityBenchmarkMatch {
	entry: CreativityBenchmarkEntry;
	matchType: Extract<CreativityBenchmarkMatchType, "direct" | "alias">;
}

interface ResolvedCreativityBenchmark {
	score: number;
	version: WritingBenchmarkVersion;
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

interface PublicBenchmarkMaps {
	creativity: Map<string, CreativityBenchmarkEntry>;
	multilinguality: Map<string, number>;
	opennessIndex: Map<string, number>;
	openSourceCategorization: Map<string, AAOpenSourceCategorization>;
	omniscienceIndex: Map<string, number>;
	omniscienceAccuracy: Map<string, number>;
	omniscienceHallucinationRate: Map<string, number>;
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

function cachedResultsPath(rootDir: string): string {
	return resolve(rootDir, "tmp", "results.json");
}

export async function loadCachedResults(
	rootDir: string,
): Promise<{ models: AAModel[]; maxes: GlobalMaxes } | null> {
	try {
		const raw = await readFile(cachedResultsPath(rootDir), "utf-8");
		const results: ResultsFile = JSON.parse(raw);
		console.log(
			`[fetch] Loaded ${results.modelCount} models from cache (fetched ${results.fetchedAt})`,
		);
		return { models: results.data, maxes: results.globalMaxes };
	} catch {
		return null;
	}
}

export async function fetchModels(apiKey: string): Promise<AAModel[]> {
	console.log("[fetch] Calling Artificial Analysis API...");
	const res = await fetch(API_URL, {
		headers: { "x-api-key": apiKey },
	});

	if (!res.ok) {
		throw new Error(`API request failed: ${res.status} ${res.statusText}`);
	}

	const json = (await res.json()) as AAApiResponse;
	console.log(`[fetch] Received ${json.data.length} models`);
	return json.data;
}

export async function fetchModelsWithCache(
	apiKey: string,
	rootDir: string,
): Promise<{ models: AAModel[]; maxes: GlobalMaxes }> {
	try {
		const cached = await loadCachedResults(rootDir);
		const models = await fetchModels(apiKey);
		const enrichedModels = await enrichModelsWithPublicBenchmarks(
			models,
			cached?.models,
		);
		const maxes = computeGlobalMaxes(enrichedModels);
		await writeResults(rootDir, enrichedModels, maxes);
		return { models: enrichedModels, maxes };
	} catch (err) {
		console.warn(`[fetch] API call failed: ${(err as Error).message}`);
		console.warn("[fetch] Attempting to load cached results...");
		const cached = await loadCachedResults(rootDir);
		if (!cached) {
			throw new Error(
				"API request failed and no cached results found in tmp/results.json. " +
					"Run a successful fetch first to populate the cache.",
			);
		}
		return cached;
	}
}

function safeMax(values: (number | null | undefined)[]): number {
	const valid = values.filter((v): v is number => v != null && v > 0);
	return valid.length > 0 ? Math.max(...valid) : 1;
}

async function enrichModelsWithPublicBenchmarks(
	models: AAModel[],
	cachedModels?: AAModel[],
): Promise<AAModel[]> {
	const cachedBySlug = new Map(
		(cachedModels ?? []).map((model) => [model.slug, model]),
	);
	let benchmarks: PublicBenchmarkMaps | null = null;
	let creativityBenchmarks: Map<string, ResolvedCreativityBenchmark> | null =
		null;

	try {
		benchmarks = await fetchPublicBenchmarks();
		console.log(
			`[fetch] Loaded ${benchmarks.creativity.size} creativity, ${benchmarks.multilinguality.size} multilingual, ${benchmarks.opennessIndex.size} openness benchmark, and ${benchmarks.omniscienceIndex.size} omniscience benchmark entries from public pages`,
		);
	} catch (err) {
		console.warn(
			`[fetch] Failed to load public benchmark pages: ${(err as Error).message}`,
		);
		console.warn(
			"[fetch] Reusing cached multilingual / omniscience scores where available...",
		);
	}

	const modelsWithCategories = models.map((model) => {
		const cachedModel = cachedBySlug.get(model.slug);
		return {
			...model,
			open_source_categorization:
				benchmarks?.openSourceCategorization.get(model.slug) ??
				cachedModel?.open_source_categorization ??
				model.open_source_categorization ??
				null,
		};
	});

	if (benchmarks) {
		creativityBenchmarks = resolveCreativityBenchmarks(
			modelsWithCategories,
			benchmarks.creativity,
		);
		const creativityStats = summarizeCreativityBenchmarks(creativityBenchmarks);
		console.log(
			`[fetch] Resolved ${creativityStats.directOrAlias} direct/alias creativity matches and ${creativityStats.similarity} similarity creativity fallbacks across ${models.length} AA models`,
		);
	}

	return modelsWithCategories.map((model) => {
		const cachedModel = cachedBySlug.get(model.slug);
		const cachedEvaluations = cachedModel?.evaluations;
		const creativityBenchmark = creativityBenchmarks?.get(model.slug) ?? null;
		const evaluations: AAEvaluations = {
			...model.evaluations,
			artificial_analysis_multilingual_index_normalized:
				benchmarks?.multilinguality.get(model.slug) ??
				cachedEvaluations?.artificial_analysis_multilingual_index_normalized ??
				null,
			artificial_analysis_openness_index:
				benchmarks?.opennessIndex.get(model.slug) ??
				cachedEvaluations?.artificial_analysis_openness_index ??
				null,
			artificial_analysis_omniscience_index_normalized:
				benchmarks?.omniscienceIndex.get(model.slug) ??
				cachedEvaluations?.artificial_analysis_omniscience_index_normalized ??
				null,
			artificial_analysis_omniscience_accuracy:
				benchmarks?.omniscienceAccuracy.get(model.slug) ??
				cachedEvaluations?.artificial_analysis_omniscience_accuracy ??
				null,
			artificial_analysis_omniscience_hallucination_rate:
				benchmarks?.omniscienceHallucinationRate.get(model.slug) ??
				cachedEvaluations?.artificial_analysis_omniscience_hallucination_rate ??
				null,
			writing_benchmark_mean_score: creativityBenchmark
				? creativityBenchmark.score
				: (cachedEvaluations?.writing_benchmark_mean_score ?? null),
			writing_benchmark_version: creativityBenchmark
				? creativityBenchmark.version
				: (cachedEvaluations?.writing_benchmark_version ?? null),
			writing_benchmark_match_type: creativityBenchmark
				? creativityBenchmark.matchType
				: (cachedEvaluations?.writing_benchmark_match_type ?? null),
			writing_benchmark_source_model_name: creativityBenchmark
				? creativityBenchmark.sourceModelName
				: (cachedEvaluations?.writing_benchmark_source_model_name ?? null),
			writing_benchmark_source_model_slug: creativityBenchmark
				? creativityBenchmark.sourceModelSlug
				: (cachedEvaluations?.writing_benchmark_source_model_slug ?? null),
			writing_benchmark_similarity: creativityBenchmark
				? creativityBenchmark.similarity
				: (cachedEvaluations?.writing_benchmark_similarity ?? null),
			writing_benchmark_similarity_tier: creativityBenchmark
				? creativityBenchmark.similarityTier
				: (cachedEvaluations?.writing_benchmark_similarity_tier ?? null),
		};

		return {
			...model,
			evaluations,
		};
	});
}

async function fetchPublicBenchmarks(): Promise<PublicBenchmarkMaps> {
	const [modelsHtml, multilingualHtml, omniscienceHtml, ...writingReadmes] =
		await Promise.all([
			fetchPageHtml(MODELS_PAGE_URL),
			fetchPageHtml(MULTILINGUAL_URL),
			fetchPageHtml(OMNISCIENCE_URL),
			...WRITING_BENCHMARK_READMES.map((source) => fetchPageHtml(source.url)),
		]);

	const modelBenchmarks = parseModelsPageBenchmarks(modelsHtml);
	const creativityBenchmarks = parseWritingBenchmarks(writingReadmes);
	const multilingualDatasets = extractLdJsonDatasets(multilingualHtml);
	const omniscienceDatasets = extractLdJsonDatasets(omniscienceHtml);

	return {
		creativity: creativityBenchmarks,
		multilinguality: parseNormalizedPropertyDataset(
			findDataset(multilingualDatasets, MULTILINGUAL_DATASET),
			"multilingual",
		),
		opennessIndex: modelBenchmarks.opennessIndex,
		openSourceCategorization: modelBenchmarks.openSourceCategorization,
		omniscienceIndex: parseNormalizedPropertyDataset(
			findDataset(omniscienceDatasets, OMNISCIENCE_INDEX_DATASET),
			"omniscience",
		),
		omniscienceAccuracy: parseNumericDataset(
			findDataset(omniscienceDatasets, OMNISCIENCE_ACCURACY_DATASET),
			"omniscienceAccuracy",
		),
		omniscienceHallucinationRate: parseNumericDataset(
			findDataset(omniscienceDatasets, OMNISCIENCE_HALLUCINATION_DATASET),
			"omniscienceHallucinationRate",
		),
	};
}

function parseWritingBenchmarks(
	readmes: string[],
): Map<string, CreativityBenchmarkEntry> {
	const benchmarks = new Map<string, CreativityBenchmarkEntry>();

	for (const [index, source] of WRITING_BENCHMARK_READMES.entries()) {
		const readme = readmes[index];
		for (const entry of parseWritingBenchmarkTable(
			readme,
			source.heading,
			source.version,
		)) {
			const key = normalizeCreativityKey(entry.modelName);
			if (!key || benchmarks.has(key)) continue;
			benchmarks.set(key, entry);
		}
	}

	return benchmarks;
}

function parseWritingBenchmarkTable(
	readme: string,
	heading: string,
	version: WritingBenchmarkVersion,
): CreativityBenchmarkEntry[] {
	const lines = readme.split(/\r?\n/);
	const start = lines.findIndex(
		(line) => line.trim().toLowerCase() === heading.toLowerCase(),
	);
	if (start === -1) return [];

	const rows: string[] = [];
	for (const line of lines.slice(start + 1)) {
		if (
			(line.startsWith("##") ||
				line.startsWith("###") ||
				line.startsWith("####")) &&
			rows.length > 0
		) {
			break;
		}

		if (line.startsWith("|")) {
			rows.push(line);
		}
	}

	return rows.slice(2).flatMap((line) => {
		const columns = line
			.trim()
			.replace(/^\|/, "")
			.replace(/\|$/, "")
			.split("|")
			.map((value) => value.trim());

		if (columns.length < 3) return [];
		const score = Number(columns[2]);
		if (!Number.isFinite(score)) return [];

		return [{ modelName: columns[1], score, version }];
	});
}

function matchCreativityBenchmark(
	model: AAModel,
	benchmarks: Map<string, CreativityBenchmarkEntry>,
): CreativityBenchmarkMatch | null {
	const alias = CREATIVITY_BENCHMARK_ALIASES[model.slug];
	if (alias) {
		const entry = benchmarks.get(alias);
		if (entry) {
			return { entry, matchType: "alias" };
		}
	}

	for (const candidate of [model.slug, model.name]) {
		const key = normalizeCreativityKey(candidate);
		if (!key) continue;

		const entry = benchmarks.get(key);
		if (entry) {
			return { entry, matchType: "direct" };
		}
	}

	return null;
}

function resolveCreativityBenchmarks(
	models: AAModel[],
	benchmarks: Map<string, CreativityBenchmarkEntry>,
): Map<string, ResolvedCreativityBenchmark> {
	const resolved = new Map<string, ResolvedCreativityBenchmark>();

	for (const model of models) {
		const match = matchCreativityBenchmark(model, benchmarks);
		if (!match) continue;

		resolved.set(model.slug, {
			score: match.entry.score,
			version: match.entry.version,
			matchType: match.matchType,
			sourceModelName: match.entry.modelName,
			sourceModelSlug: match.matchType === "direct" ? model.slug : null,
			similarity: null,
			similarityTier: null,
		});
	}

	if (resolved.size === 0) {
		return resolved;
	}

	const donorModels = models.filter((model) => resolved.has(model.slug));
	const featureMaxes = computeCreativitySimilarityFeatureMaxes(models);

	for (const model of models) {
		if (resolved.has(model.slug)) continue;

		const similarityMatch = matchCreativityBySimilarity(
			model,
			donorModels,
			resolved,
			featureMaxes,
		);
		if (similarityMatch) {
			resolved.set(model.slug, similarityMatch);
		}
	}

	return resolved;
}

function summarizeCreativityBenchmarks(
	benchmarks: Map<string, ResolvedCreativityBenchmark>,
): { directOrAlias: number; similarity: number } {
	let directOrAlias = 0;
	let similarity = 0;

	for (const benchmark of benchmarks.values()) {
		if (benchmark.matchType === "similarity") {
			similarity += 1;
		} else {
			directOrAlias += 1;
		}
	}

	return { directOrAlias, similarity };
}

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

		const sourceScore = clampUnit(sourceBenchmark.score / 10);
		const similarity = clampUnit(
			(1 - bestMatch.distance) * CREATIVITY_SIMILARITY_TIER_DAMPING[group.tier],
		);
		const estimatedScore =
			CREATIVITY_NEUTRAL_SCORE +
			similarity * (sourceScore - CREATIVITY_NEUTRAL_SCORE);

		return {
			score: roundToPrecision(estimatedScore * 10, 3),
			version: sourceBenchmark.version,
			matchType: "similarity",
			sourceModelName: bestMatch.candidate.name,
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

function parseModelsPageBenchmarks(html: string): {
	opennessIndex: Map<string, number>;
	openSourceCategorization: Map<string, AAOpenSourceCategorization>;
} {
	const opennessIndex = new Map<string, number>();
	const openSourceCategorization = new Map<
		string,
		AAOpenSourceCategorization
	>();
	const start = html.indexOf('{\\"additional_text\\":');

	if (start === -1) {
		return { opennessIndex, openSourceCategorization };
	}

	const chunks = html
		.slice(start)
		.split('},{\\"additional_text\\":')
		.map((chunk, index) =>
			index === 0 ? chunk : `{\\"additional_text\\":${chunk}`,
		);

	for (const chunk of chunks) {
		const modelUrl = parseEscapedStringField(chunk, "model_url");
		if (!modelUrl?.startsWith("/models/")) {
			continue;
		}

		const slug = modelUrl.slice("/models/".length);
		const category = parseEscapedStringField(
			chunk,
			"open_source_categorization",
		);
		const openness = parseEscapedNumericField(chunk, "opennessIndex");

		if (isOpenSourceCategorization(category)) {
			openSourceCategorization.set(slug, category);
		}

		if (openness != null) {
			opennessIndex.set(slug, openness);
		}
	}

	return { opennessIndex, openSourceCategorization };
}

async function fetchPageHtml(url: string): Promise<string> {
	const res = await fetch(url);
	if (!res.ok) {
		throw new Error(
			`Public benchmark request failed: ${res.status} ${res.statusText} (${url})`,
		);
	}
	return await res.text();
}

function extractLdJsonDatasets(html: string): JsonObject[] {
	const datasets: JsonObject[] = [];
	const pattern = /<script type="application\/ld\+json">([\s\S]*?)<\/script>/g;

	for (const match of html.matchAll(pattern)) {
		const raw = match[1]?.trim();
		if (!raw) continue;

		try {
			const parsed = JSON.parse(raw);
			if (parsed && typeof parsed === "object") {
				datasets.push(parsed as JsonObject);
			}
		} catch {
			// Ignore malformed or non-dataset JSON-LD blocks.
		}
	}

	return datasets;
}

function findDataset(datasets: JsonObject[], name: string): JsonObject | null {
	for (const dataset of datasets) {
		if (dataset["@type"] === "Dataset" && dataset.name === name) {
			return dataset;
		}
	}

	return null;
}

function parseNormalizedPropertyDataset(
	dataset: JsonObject | null,
	propertyKey: string,
): Map<string, number> {
	const result = new Map<string, number>();
	if (!dataset) return result;

	const rows = Array.isArray(dataset.data) ? dataset.data : [];
	for (const row of rows) {
		if (!row || typeof row !== "object") continue;

		const detailsUrl =
			typeof row.detailsUrl === "string" ? row.detailsUrl : null;
		const slug = extractSlugFromDetailsUrl(detailsUrl);
		if (!slug) continue;

		const properties = Array.isArray((row as JsonObject)[propertyKey])
			? ((row as JsonObject)[propertyKey] as unknown[])
			: [];
		const values = properties
			.map((entry) => {
				if (!entry || typeof entry !== "object") return null;
				const value = (entry as JsonObject).value;
				return typeof value === "number" ? clampUnit(value) : null;
			})
			.filter((value): value is number => value != null);

		if (values.length > 0) {
			result.set(
				slug,
				values.reduce((sum, value) => sum + value, 0) / values.length,
			);
		}
	}

	return result;
}

function parseNumericDataset(
	dataset: JsonObject | null,
	valueKey: string,
): Map<string, number> {
	const result = new Map<string, number>();
	if (!dataset) return result;

	const rows = Array.isArray(dataset.data) ? dataset.data : [];
	for (const row of rows) {
		if (!row || typeof row !== "object") continue;

		const detailsUrl =
			typeof row.detailsUrl === "string" ? row.detailsUrl : null;
		const slug = extractSlugFromDetailsUrl(detailsUrl);
		const value = (row as JsonObject)[valueKey];

		if (slug && typeof value === "number") {
			result.set(slug, clampUnit(value));
		}
	}

	return result;
}

function extractSlugFromDetailsUrl(detailsUrl: string | null): string | null {
	if (!detailsUrl) return null;
	const match = detailsUrl.match(/^\/models\/([^/]+)\/providers$/);
	return match?.[1] ?? null;
}

function parseEscapedStringField(
	chunk: string,
	fieldName: string,
): string | null {
	const match = chunk.match(
		new RegExp(`\\\\"${fieldName}\\\\":\\\\"([^\\\\"]+)\\\\"`),
	);
	return match?.[1] ?? null;
}

function parseEscapedNumericField(
	chunk: string,
	fieldName: string,
): number | null {
	const match = chunk.match(new RegExp(`\\\\"${fieldName}\\\\":([0-9.]+)`));
	if (!match) return null;

	const value = Number(match[1]);
	return Number.isFinite(value) ? value : null;
}

function isOpenSourceCategorization(
	value: string | null,
): value is AAOpenSourceCategorization {
	return (
		value === "Open Weights (License Required for Commercial Use)" ||
		value === "Open Weights (Permissive License)" ||
		value === "Proprietary"
	);
}

function normalizeCreativityKey(value: string): string {
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
		.replace(/preview/g, "")
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

export function computeGlobalMaxes(models: AAModel[]): GlobalMaxes {
	return {
		coding_index: safeMax(
			models.map((m) => m.evaluations.artificial_analysis_coding_index),
		),
		livecodebench: safeMax(models.map((m) => m.evaluations.livecodebench)),
		scicode: safeMax(models.map((m) => m.evaluations.scicode)),
		terminalbench_hard: safeMax(
			models.map((m) => m.evaluations.terminalbench_hard),
		),
		math_index: safeMax(
			models.map((m) => m.evaluations.artificial_analysis_math_index),
		),
		aime: safeMax(models.map((m) => m.evaluations.aime)),
		aime_25: safeMax(models.map((m) => m.evaluations.aime_25)),
		math_500: safeMax(models.map((m) => m.evaluations.math_500)),
		hle: safeMax(models.map((m) => m.evaluations.hle)),
		gpqa: safeMax(models.map((m) => m.evaluations.gpqa)),
		lcr: safeMax(models.map((m) => m.evaluations.lcr)),
		mmlu_pro: safeMax(models.map((m) => m.evaluations.mmlu_pro)),
		intelligence_index: safeMax(
			models.map((m) => m.evaluations.artificial_analysis_intelligence_index),
		),
		tau2: safeMax(models.map((m) => m.evaluations.tau2)),
		ifbench: safeMax(models.map((m) => m.evaluations.ifbench)),
		price_1m_blended_3_to_1: safeMax(
			models.map((m) => m.pricing.price_1m_blended_3_to_1),
		),
		median_output_tokens_per_second: safeMax(
			models.map((m) => m.median_output_tokens_per_second),
		),
	};
}

export async function writeResults(
	rootDir: string,
	models: AAModel[],
	maxes: GlobalMaxes,
): Promise<void> {
	const tmpDir = resolve(rootDir, "tmp");
	await mkdir(tmpDir, { recursive: true });

	const results: ResultsFile = {
		fetchedAt: new Date().toISOString(),
		modelCount: models.length,
		globalMaxes: maxes,
		data: models,
	};

	const outPath = resolve(tmpDir, "results.json");
	await writeFile(outPath, JSON.stringify(results, null, 2));
	console.log(`[fetch] Wrote results to ${outPath}`);
}
