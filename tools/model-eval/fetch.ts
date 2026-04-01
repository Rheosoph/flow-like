import type { AAApiResponse, AAModel, GlobalMaxes, ResultsFile } from "./types";
import { resolve } from "path";
import { mkdir, readFile, writeFile } from "fs/promises";

const API_URL = "https://artificialanalysis.ai/api/v2/data/llms/models";

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
		const models = await fetchModels(apiKey);
		const maxes = computeGlobalMaxes(models);
		await writeResults(rootDir, models, maxes);
		return { models, maxes };
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

export function computeGlobalMaxes(models: AAModel[]): GlobalMaxes {
	return {
		coding_index: safeMax(models.map((m) => m.evaluations.artificial_analysis_coding_index)),
		livecodebench: safeMax(models.map((m) => m.evaluations.livecodebench)),
		scicode: safeMax(models.map((m) => m.evaluations.scicode)),
		terminalbench_hard: safeMax(models.map((m) => m.evaluations.terminalbench_hard)),
		math_index: safeMax(models.map((m) => m.evaluations.artificial_analysis_math_index)),
		aime: safeMax(models.map((m) => m.evaluations.aime)),
		aime_25: safeMax(models.map((m) => m.evaluations.aime_25)),
		math_500: safeMax(models.map((m) => m.evaluations.math_500)),
		hle: safeMax(models.map((m) => m.evaluations.hle)),
		gpqa: safeMax(models.map((m) => m.evaluations.gpqa)),
		lcr: safeMax(models.map((m) => m.evaluations.lcr)),
		mmlu_pro: safeMax(models.map((m) => m.evaluations.mmlu_pro)),
		intelligence_index: safeMax(models.map((m) => m.evaluations.artificial_analysis_intelligence_index)),
		tau2: safeMax(models.map((m) => m.evaluations.tau2)),
		ifbench: safeMax(models.map((m) => m.evaluations.ifbench)),
		price_1m_blended_3_to_1: safeMax(models.map((m) => m.pricing.price_1m_blended_3_to_1)),
		median_output_tokens_per_second: safeMax(models.map((m) => m.median_output_tokens_per_second)),
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
