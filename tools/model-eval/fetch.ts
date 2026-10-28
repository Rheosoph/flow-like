import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { parsePublicModelBenchmarks } from "./public-benchmarks";
import type {
	AAApiResponse,
	AAEvaluations,
	AAModel,
	AAOpenSourceCategorization,
	GlobalMaxes,
	ResultsFile,
} from "./types";
import { isCatalogEstimateSlug } from "./types";

import {
	type CreativityBenchmarkEntry,
	type ResolvedCreativityBenchmark,
	WRITING_BENCHMARK_READMES,
	parseWritingBenchmarks,
	resolveCreativityBenchmarks,
	summarizeCreativityBenchmarks,
} from "./writing";

const API_URL = "https://artificialanalysis.ai/api/v2/data/llms/models";
const MODELS_PAGE_URL = "https://artificialanalysis.ai/models";
const MULTILINGUAL_URL = "https://artificialanalysis.ai/models/multilingual";
const OMNISCIENCE_URL = "https://artificialanalysis.ai/evaluations/omniscience";
const MULTILINGUAL_DATASET = "Multilingual Index Across Languages (Normalized)";
const OMNISCIENCE_INDEX_DATASET =
	"AA-Omniscience Index Across Domains (Normalized)";
const OMNISCIENCE_ACCURACY_DATASET = "AA-Omniscience Accuracy";
const OMNISCIENCE_HALLUCINATION_DATASET = "AA-Omniscience Hallucination Rate";
const OPENNESS_INDEX_DATASET = "Artificial Analysis Openness Index: Score";

type JsonObject = Record<string, unknown>;

interface PublicBenchmarkMaps {
	creativity: Map<string, CreativityBenchmarkEntry>;
	multilinguality: Map<string, number>;
	multilingualIndex: Map<string, number>;
	automationBench: Map<string, number>;
	opennessIndex: Map<string, number>;
	openSourceCategorization: Map<string, AAOpenSourceCategorization>;
	omniscienceIndex: Map<string, number>;
	omniscienceAccuracy: Map<string, number>;
	omniscienceHallucinationRate: Map<string, number>;
}

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
		// Older caches predate some benchmark fields; rebuild their denominators.
		const models = results.data.filter(
			(model) => !isCatalogEstimateSlug(model.slug),
		);
		return { models, maxes: computeGlobalMaxes(models) };
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
	return json.data.filter((model) => !isCatalogEstimateSlug(model.slug));
}

export async function fetchModelsWithCache(
	apiKey: string,
	rootDir: string,
	modelSlugs: readonly string[] = [],
): Promise<{ models: AAModel[]; maxes: GlobalMaxes }> {
	try {
		const cached = await loadCachedResults(rootDir);
		const models = await fetchModels(apiKey);
		const availableSlugs = new Set(models.map((model) => model.slug));
		const enrichedModels = await enrichModelsWithPublicBenchmarks(
			models,
			cached?.models,
			modelSlugs.filter((slug) => availableSlugs.has(slug)),
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
	const valid = values.filter(
		(v): v is number => v != null && Number.isFinite(v) && v > 0,
	);
	return valid.length > 0 ? Math.max(...valid) : 1;
}

async function enrichModelsWithPublicBenchmarks(
	models: AAModel[],
	cachedModels?: AAModel[],
	modelSlugs: readonly string[] = [],
): Promise<AAModel[]> {
	const cachedBySlug = new Map(
		(cachedModels ?? []).map((model) => [model.slug, model]),
	);
	let benchmarks: PublicBenchmarkMaps | null = null;
	let creativityBenchmarks: Map<string, ResolvedCreativityBenchmark> | null =
		null;

	try {
		benchmarks = await fetchPublicBenchmarks(modelSlugs);
		const multilingualCount = new Set([
			...benchmarks.multilinguality.keys(),
			...benchmarks.multilingualIndex.keys(),
		]).size;
		const omniscienceCount = new Set([
			...benchmarks.omniscienceIndex.keys(),
			...benchmarks.omniscienceAccuracy.keys(),
			...benchmarks.omniscienceHallucinationRate.keys(),
		]).size;
		console.log(
			`[fetch] Loaded ${benchmarks.creativity.size} creativity, ${multilingualCount} multilingual, ${benchmarks.opennessIndex.size} openness, ${omniscienceCount} omniscience, and ${benchmarks.automationBench.size} automation benchmark entries from public pages`,
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
		const currentMultilingual =
			benchmarks?.multilingualIndex.get(model.slug) ??
			model.evaluations.artificial_analysis_multilingual_index;
		const currentAccuracy =
			benchmarks?.omniscienceAccuracy.get(model.slug) ??
			model.evaluations.artificial_analysis_omniscience_accuracy;
		const currentHallucination =
			benchmarks?.omniscienceHallucinationRate.get(model.slug) ??
			model.evaluations.artificial_analysis_omniscience_hallucination_rate;
		const evaluations: AAEvaluations = {
			...model.evaluations,
			automationbench_aa:
				benchmarks?.automationBench.get(model.slug) ??
				model.evaluations.automationbench_aa ??
				cachedEvaluations?.automationbench_aa ??
				null,
			artificial_analysis_multilingual_index:
				currentMultilingual ??
				cachedEvaluations?.artificial_analysis_multilingual_index ??
				null,
			artificial_analysis_multilingual_index_normalized:
				benchmarks?.multilinguality.get(model.slug) ??
				model.evaluations.artificial_analysis_multilingual_index_normalized ??
				(currentMultilingual == null
					? cachedEvaluations?.artificial_analysis_multilingual_index_normalized
					: null) ??
				null,
			artificial_analysis_openness_index:
				benchmarks?.opennessIndex.get(model.slug) ??
				cachedEvaluations?.artificial_analysis_openness_index ??
				null,
			artificial_analysis_omniscience_index_normalized:
				benchmarks?.omniscienceIndex.get(model.slug) ??
				model.evaluations.artificial_analysis_omniscience_index_normalized ??
				(currentAccuracy == null && currentHallucination == null
					? cachedEvaluations?.artificial_analysis_omniscience_index_normalized
					: null) ??
				null,
			artificial_analysis_omniscience_accuracy:
				currentAccuracy ??
				cachedEvaluations?.artificial_analysis_omniscience_accuracy ??
				null,
			artificial_analysis_omniscience_hallucination_rate:
				currentHallucination ??
				cachedEvaluations?.artificial_analysis_omniscience_hallucination_rate ??
				null,
			writing_benchmark_mean_score: creativityBenchmark
				? creativityBenchmark.meanScore
				: (cachedEvaluations?.writing_benchmark_mean_score ?? null),
			writing_benchmark_comparison_score: creativityBenchmark
				? creativityBenchmark.comparisonScore
				: (cachedEvaluations?.writing_benchmark_comparison_score ?? null),
			writing_benchmark_estimated_win_chance: creativityBenchmark
				? creativityBenchmark.estimatedWinChance
				: (cachedEvaluations?.writing_benchmark_estimated_win_chance ?? null),
			writing_benchmark_creativity_estimate: creativityBenchmark
				? creativityBenchmark.creativityEstimate
				: (cachedEvaluations?.writing_benchmark_creativity_estimate ?? null),
			writing_benchmark_source_url: creativityBenchmark
				? creativityBenchmark.sourceUrl
				: (cachedEvaluations?.writing_benchmark_source_url ?? null),
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

async function fetchPublicBenchmarks(
	modelSlugs: readonly string[] = [],
): Promise<PublicBenchmarkMaps> {
	const sources = [
		MODELS_PAGE_URL,
		MULTILINGUAL_URL,
		OMNISCIENCE_URL,
		...WRITING_BENCHMARK_READMES.map((source) => source.url),
	];

	const pages = await fetchPages(sources);
	// Leaderboards render only a subset of models. Fetch linked catalog models
	// individually so an off-screen row can still receive current benchmarks.
	const modelPages = await fetchPages(
		[...new Set(modelSlugs)].map(
			(slug) => `${MODELS_PAGE_URL}/${encodeURIComponent(slug)}`,
		),
	);

	const [modelsHtml, multilingualHtml, omniscienceHtml, ...writingReadmes] =
		pages;

	const creativityBenchmarks = parseWritingBenchmarks(writingReadmes);
	const modelsDatasets = extractLdJsonDatasets(modelsHtml);
	const multilingualDatasets = extractLdJsonDatasets(multilingualHtml);
	const omniscienceDatasets = extractLdJsonDatasets(omniscienceHtml);

	const maps: PublicBenchmarkMaps = {
		creativity: creativityBenchmarks,
		multilingualIndex: new Map(),
		automationBench: new Map(),
		multilinguality: parseNormalizedPropertyDataset(
			findDataset(multilingualDatasets, MULTILINGUAL_DATASET),
			"multilingual",
		),
		// Openness is published on a 0-100 scale.
		opennessIndex: parseNumericDataset(
			findDataset(modelsDatasets, OPENNESS_INDEX_DATASET),
			"opennessIndex",
			false,
		),
		openSourceCategorization: new Map(),
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
	for (const html of [
		modelsHtml,
		multilingualHtml,
		omniscienceHtml,
		...modelPages,
	]) {
		for (const [slug, row] of parsePublicModelBenchmarks(html)) {
			if (row.opennessIndex !== undefined)
				maps.opennessIndex.set(slug, row.opennessIndex);
			if (row.openSourceCategorization !== undefined)
				maps.openSourceCategorization.set(slug, row.openSourceCategorization);
			if (row.omniscienceAccuracy !== undefined)
				maps.omniscienceAccuracy.set(slug, row.omniscienceAccuracy);
			if (row.omniscienceHallucinationRate !== undefined)
				maps.omniscienceHallucinationRate.set(
					slug,
					row.omniscienceHallucinationRate,
				);
			if (row.automationBench !== undefined)
				maps.automationBench.set(slug, row.automationBench);
			if (row.multilingualIndex !== undefined)
				maps.multilingualIndex.set(slug, row.multilingualIndex);
		}
	}
	return maps;
}

async function fetchPages(urls: string[]): Promise<string[]> {
	const pages: string[] = [];
	for (let offset = 0; offset < urls.length; offset += 5) {
		const batch = urls.slice(offset, offset + 5);
		const settled = await Promise.allSettled(batch.map(fetchPageHtml));
		for (const [index, result] of settled.entries()) {
			if (result.status === "fulfilled") {
				pages.push(result.value);
			} else {
				// A retired source must not block refreshes from the remaining pages.
				console.warn(
					`[fetch] Skipping ${batch[index]}: ${(result.reason as Error).message}`,
				);
				pages.push("");
			}
		}
	}
	return pages;
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
	clamp = true,
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
			result.set(slug, clamp ? clampUnit(value) : value);
		}
	}

	return result;
}

function extractSlugFromDetailsUrl(detailsUrl: string | null): string | null {
	if (!detailsUrl) return null;
	// Datasets link a model either directly or through its providers tab, and
	// the site moved several of them from the second form to the first.
	const match = detailsUrl.match(/^\/models\/([^/]+)(?:\/providers)?$/);
	return match?.[1] ?? null;
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
		terminalbench_v2_1: safeMax(
			models.map((m) => m.evaluations.terminalbench_v2_1),
		),
		terminalbench_v4_0: safeMax(
			models.map((m) => m.evaluations.terminalbench_v4_0),
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
		tau_banking: safeMax(models.map((m) => m.evaluations.tau_banking)),
		automationbench_aa: safeMax(
			models.map((m) => m.evaluations.automationbench_aa),
		),
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
