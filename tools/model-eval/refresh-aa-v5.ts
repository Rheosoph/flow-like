#!/usr/bin/env bun
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { config } from "dotenv";
import {
	type AAV5Catalog,
	type AAV5Model,
	AA_V5_VERSION,
	fetchAAV5Catalog,
} from "./aa-v5";
import { refreshBenchmarkDescription } from "./aa-v5-metadata";
import {
	type JsonRecord,
	type ScorePair,
	type ScorePrior,
	calibrateScores,
	estimateV5,
	legacyEvaluations,
	migrateClassification,
	migrateEvaluations,
	record,
	score,
} from "./aa-v5-scoring";
import { catalogReleaseDate, savedBitFromEvents } from "./update-catalog";

export interface LanguageBit extends JsonRecord {
	id: string;
	type: string;
	model_slug?: string | null;
	model_evaluation?: JsonRecord | null;
	parameters?: JsonRecord | null;
	meta: Record<string, JsonRecord>;
}
interface HistoricalModel {
	slug: string;
	name: string;
	evaluations: JsonRecord;
}
export interface RefreshPlan {
	bit: LanguageBit;
	slug: string;
	model: JsonRecord;
	parameters: JsonRecord;
	englishMeta?: JsonRecord;
}
const CORRECTED_QWEN = "ngbc8xvkrv5mtvcjq81w9n6v";
const UNCENSORED_QWEN = "m3mnsg6v81uvikfmr6nssurl";

export function evaluationSlug(bit: LanguageBit): string {
	if (bit.id === CORRECTED_QWEN) {
		if (!/Qwen3-4B-Instruct-2507/i.test(String(bit.file_name)))
			throw new Error("Qwen3 4B artifact identity changed; review mapping");
		return "qwen3-4b-2507-instruct";
	}
	if (bit.id === UNCENSORED_QWEN)
		return "flow-like-estimate-qwen3-8-27b-uncensored";
	return bit.model_slug ?? `flow-like-estimate-${bit.id}`;
}
function normalizedName(name: unknown): string {
	return String(name ?? "")
		.toLowerCase()
		.replace(/[^a-z0-9]/g, "");
}
function previousEvaluations(bit: LanguageBit): JsonRecord {
	return legacyEvaluations(record(bit.model_evaluation?.evaluations));
}
function currentIndex(evaluations: JsonRecord): boolean {
	const version = /^v?(\d+)(?:\.|$)/i.exec(
		String(
			evaluations.artificial_analysis_intelligence_index_version ?? "",
		).trim(),
	);
	return (
		(version !== null && Number(version[1]) >= 5) ||
		score(evaluations.hle_diamond) ||
		score(evaluations.critpt_agentic_hard)
	);
}

export function refreshEnglishMetadata(
	meta: unknown,
	evaluations: JsonRecord,
): JsonRecord | undefined {
	const english = record(meta);
	if (
		typeof english.name !== "string" ||
		typeof english.description !== "string" ||
		(english.long_description != null &&
			typeof english.long_description !== "string")
	) {
		throw new Error(
			"English model metadata must contain string name, description and optional long_description",
		);
	}
	const description = refreshBenchmarkDescription(
		String(english.long_description ?? ""),
		evaluations,
	);
	return description === english.long_description
		? undefined
		: { ...english, long_description: description };
}
function canonicalJson(value: unknown): string {
	const normalize = (item: unknown): unknown => {
		if (Array.isArray(item)) return item.map(normalize);
		if (!item || typeof item !== "object") return item;
		return Object.fromEntries(
			Object.entries(item)
				.sort(([a], [b]) => a.localeCompare(b))
				.map(([key, entry]) => [key, normalize(entry)]),
		);
	};
	return JSON.stringify(normalize(value));
}

export function assertBitUnchanged(
	planned: LanguageBit,
	current: LanguageBit,
	compareEvaluation = true,
): void {
	const identity = [
		"id",
		"type",
		"model_slug",
		"file_name",
		"download_link",
		"repository",
		"parameters",
	];
	if (
		identity.some(
			(key) => canonicalJson(planned[key]) !== canonicalJson(current[key]),
		) ||
		(compareEvaluation &&
			canonicalJson(planned.model_evaluation) !==
				canonicalJson(current.model_evaluation))
	) {
		throw new Error(
			`Bit ${planned.id} changed after planning; stopped before overwriting it`,
		);
	}
}

/** Only identical model variants with observed older indices can train the conversion. */
export function calibrationPairs(
	bits: LanguageBit[],
	models: Map<string, AAV5Model>,
): ScorePair[] {
	const pairs = new Map<string, ScorePair>();
	for (const bit of bits) {
		const slug = evaluationSlug(bit);
		if (slug !== bit.model_slug || slug === "qwen4-max") continue;
		const current = models.get(slug);
		const old = previousEvaluations(bit);
		if (
			!current ||
			!score(current.intelligenceIndex) ||
			!score(old.artificial_analysis_intelligence_index)
		)
			continue;
		if (currentIndex(old)) continue;
		const priorName =
			record(bit.parameters?.legacy_benchmark_identity).name ??
			bit.model_evaluation?.name;
		if (normalizedName(priorName) !== normalizedName(current.name)) continue;
		if (
			old.benchmark_reasoning_effort != null &&
			normalizedName(old.benchmark_reasoning_effort) !==
				normalizedName(current.effort?.slug)
		)
			continue;
		const pair = {
			slug,
			creator: current.creator.slug,
			previous: old.artificial_analysis_intelligence_index,
			current: current.intelligenceIndex,
			source: current.sourceUrl,
		};
		if (
			pairs.has(slug) &&
			canonicalJson(pairs.get(slug)) !== canonicalJson(pair)
		)
			throw new Error(`Conflicting legacy results for ${slug}`);
		pairs.set(slug, pair);
	}
	return [...pairs.values()];
}

export function scorePrior(
	bit: LanguageBit,
	historical: HistoricalModel[],
): ScorePrior | undefined {
	const old = previousEvaluations(bit);
	const slug = evaluationSlug(bit);
	const source = `https://api.flow-like.com/api/v1/bit/${bit.id}?language=en`;
	// This artifact had been linked to a different reasoning variant. Use the exact model's snapshot.
	if (bit.id === CORRECTED_QWEN) {
		const exact = historical.find((m) => m.slug === slug);
		const value = exact?.evaluations.artificial_analysis_intelligence_index;
		return score(value) && !currentIndex(record(exact?.evaluations))
			? {
					score: value,
					low: value,
					high: value,
					source: `https://artificialanalysis.ai/models/${slug}`,
					basis:
						"Input is the exact non-thinking Qwen3-4B-Instruct-2507 result from a historical snapshot, whose precise pre-v5 version was not recorded.",
				}
			: undefined;
	}
	const original = old.artificial_analysis_intelligence_index;
	const historicalScore = record(old.historical_intelligence_cache).score;
	if (score(historicalScore))
		return {
			score: historicalScore,
			low: historicalScore,
			high: historicalScore,
			source: String(
				record(old.historical_intelligence_cache).source ?? source,
			),
			basis:
				"Input is an archived pre-v5 measurement; its exact older index version was not recorded.",
		};
	if (score(original) && !currentIndex(old)) {
		const modified =
			bit.id === UNCENSORED_QWEN || slug.endsWith("qwen3-8-27b-uncensored");
		return {
			score: original,
			low: Math.max(0, original - (modified ? 7 : 0)),
			high: Math.min(100, original + (modified ? 7 : 0)),
			source,
			basis: modified
				? "Input is the base Qwen3.8 27B result used as a proxy for this modified model, with an additional seven-point prior margin. The uncensored artifact has no direct evaluation."
				: "Input is the archived catalog measurement; its exact older index version was not recorded.",
		};
	}
	const estimate = record(old.intelligence_estimate);
	if (
		score(estimate.score) &&
		score(estimate.low) &&
		score(estimate.high) &&
		!currentIndex(old)
	) {
		return {
			score: estimate.score,
			low: estimate.low,
			high: estimate.high,
			source: Array.isArray(estimate.sources)
				? String(estimate.sources[0] ?? source)
				: source,
			basis:
				"Input was already a catalog estimate on the legacy scale. Its full prior range is propagated into this estimate.",
		};
	}
	return undefined;
}

export function planRefresh(
	bits: LanguageBit[],
	source: AAV5Catalog,
	historical: HistoricalModel[] = [],
) {
	if (source.version !== AA_V5_VERSION)
		throw new Error("Unexpected source index version");
	if (new Set(bits.map((bit) => bit.id)).size !== bits.length)
		throw new Error("Duplicate catalog Bit IDs");
	const models = new Map<string, AAV5Model>();
	for (const model of source.models) {
		if (model.version !== source.version)
			throw new Error(`Unexpected source index version for ${model.slug}`);
		if (
			models.has(model.slug) &&
			canonicalJson(models.get(model.slug)) !== canonicalJson(model)
		)
			throw new Error(`Conflicting source models for ${model.slug}`);
		models.set(model.slug, model);
	}
	const calibration = calibrateScores(calibrationPairs(bits, models));
	const verifiedAt = source.pages[0]?.fetchedAt;
	if (!verifiedAt) throw new Error("Missing source fetch timestamp");
	const plans: RefreshPlan[] = [];
	for (const bit of bits) {
		if (!["Llm", "Vlm"].includes(bit.type))
			throw new Error(
				"Only language and vision-language models belong in this refresh",
			);
		const slug = evaluationSlug(bit);
		const aa = models.get(slug);
		const prior = scorePrior(bit, historical);
		const estimate =
			!score(aa?.intelligenceIndex) && prior
				? estimateV5(prior, calibration, verifiedAt)
				: undefined;
		const previous = bit.model_evaluation ?? {};
		const evaluations = migrateEvaluations(
			record(previous.evaluations),
			aa,
			estimate,
			verifiedAt,
		);
		const parameters = migrateClassification(
			bit.parameters ?? {},
			aa,
			verifiedAt,
		);
		parameters.legacy_benchmark_identity = bit.parameters
			?.legacy_benchmark_identity ?? {
			slug: bit.model_slug ?? null,
			name: previous.name ?? null,
		};
		plans.push({
			bit,
			slug,
			parameters,
			englishMeta: refreshEnglishMetadata(bit.meta.en, evaluations),
			model: {
				...previous,
				slug,
				name:
					aa?.name ??
					(slug === bit.model_slug ? previous.name : bit.meta.en?.name) ??
					slug,
				release_date: catalogReleaseDate(
					previous.release_date ?? aa?.releaseDate,
				),
				creator_name: previous.creator_name ?? aa?.creator.name ?? "Unknown",
				creator_slug: previous.creator_slug ?? aa?.creator.slug ?? "unknown",
				evaluations,
			},
		});
	}
	const plannedModels = new Map<string, string>();
	for (const plan of plans) {
		const json = canonicalJson(plan.model);
		if (plannedModels.has(plan.slug) && plannedModels.get(plan.slug) !== json)
			throw new Error(`Conflicting planned evaluations for ${plan.slug}`);
		plannedModels.set(plan.slug, json);
	}
	return { plans, calibration };
}

function validateSourceSnapshot(source: AAV5Catalog) {
	if (
		source.version !== AA_V5_VERSION ||
		!Array.isArray(source.models) ||
		!Array.isArray(source.pages)
	)
		throw new Error("Invalid AA source snapshot");
	for (const row of [...source.models, ...source.pages]) {
		const age = Date.now() - Date.parse(row.fetchedAt);
		if (
			!Number.isFinite(age) ||
			age < -60_000 ||
			age > 24 * 60 * 60_000 ||
			new URL(row.sourceUrl).origin !== "https://artificialanalysis.ai"
		)
			throw new Error(
				"AA snapshot must contain source pages fetched within the last 24 hours",
			);
	}
}

export async function main() {
	if (process.argv.includes("--help")) {
		console.log(
			"Usage: bun tools/model-eval/refresh-aa-v5.ts [--apply --snapshot=/tmp/aa-before.json] [--source=/tmp/fresh-aa-source.json]\nDefaults to a read-only preview. Refreshes AA v5.0.1 and calibrated estimates through the deployed admin API. --source accepts an explicitly selected snapshot under 24 hours old. FLOW_LIKE_BASE_URL defaults to https://api.flow-like.com; writes require FLOW_LIKE_PAT.",
		);
		return;
	}
	config({ path: resolve(import.meta.dir, "../../.env"), quiet: true });
	const apply = process.argv.includes("--apply");
	if (apply && process.argv.includes("--dry-run"))
		throw new Error("--apply and --dry-run cannot be combined");
	const option = (name: string) =>
		process.argv
			.find((arg) => arg.startsWith(`${name}=`))
			?.slice(name.length + 1);
	const snapshot = option("--snapshot") ?? "";
	if (apply && !snapshot)
		throw new Error(
			"Provide --snapshot to archive the affected records before applying",
		);
	const token = process.env.FLOW_LIKE_PAT;
	if (apply && !token) throw new Error("FLOW_LIKE_PAT is required");
	const base = new URL(
		process.env.FLOW_LIKE_BASE_URL ?? "https://api.flow-like.com",
	);
	if (base.protocol !== "https:" || base.username || base.password)
		throw new Error("Catalog base must be HTTPS without URL credentials");
	const api = `${base.origin}${base.pathname.replace(/\/$/, "").replace(/\/api\/v1$/, "")}/api/v1`;
	async function request(path: string, method = "GET", body?: unknown) {
		const response = await fetch(`${api}/${path}`, {
			method,
			headers: {
				"content-type": "application/json",
				...(token ? { authorization: `Bearer ${token}` } : {}),
			},
			body: body === undefined ? undefined : JSON.stringify(body),
			redirect: "error",
			signal: AbortSignal.timeout(60_000),
		});
		if (!response.ok)
			throw new Error(`${method} ${path}: HTTP ${response.status}`);
		return response;
	}
	let readSequence = 0;
	async function freshBit(id: string): Promise<LanguageBit> {
		const language = `aa-refresh-${Date.now()}-${readSequence++}`;
		const bit = (await (
			await request(`bit/${id}?language=${language}`)
		).json()) as LanguageBit;
		const meta = { ...bit.meta };
		// Some deployed readers duplicate the English fallback under the requested language.
		delete meta[language];
		return { ...bit, meta };
	}
	const bits: LanguageBit[] = [];
	for (let offset = 0; ; offset += 100) {
		const page = (await (
			await request(`bit?language=aa-refresh-${Date.now()}`, "POST", {
				bit_types: ["Llm", "Vlm"],
				limit: 100,
				offset,
			})
		).json()) as LanguageBit[];
		if (!Array.isArray(page)) throw new Error("Invalid catalog response");
		bits.push(...page);
		if (page.length < 100) break;
		if (offset >= 10_000)
			throw new Error("Catalog pagination exceeded its safety bound");
	}
	const sourcePath = option("--source");
	const source: AAV5Catalog = sourcePath
		? JSON.parse(await readFile(sourcePath, "utf8"))
		: await fetchAAV5Catalog([
				...new Set(
					bits
						.map(evaluationSlug)
						.filter((slug) => !slug.startsWith("flow-like-estimate-")),
				),
			]);
	validateSourceSnapshot(source);
	let historical: HistoricalModel[] = [];
	try {
		historical =
			JSON.parse(
				await readFile(
					resolve(import.meta.dir, "../../tmp/results.json"),
					"utf8",
				),
			).data ?? [];
	} catch {
		/* Exact historical priors are optional; no fetch fallback uses them. */
	}
	const { plans, calibration } = planRefresh(bits, source, historical);
	const summary = plans.map((plan) => {
		const e = record(plan.model.evaluations);
		return {
			id: plan.bit.id,
			name: plan.bit.meta.en?.name ?? plan.bit.model_evaluation?.name,
			slug: plan.slug,
			status: e.benchmark_status,
			score:
				e.artificial_analysis_intelligence_index ??
				record(e.intelligence_estimate).score ??
				null,
			range:
				e.benchmark_status === "estimated"
					? [
							record(e.intelligence_estimate).low,
							record(e.intelligence_estimate).high,
						]
					: undefined,
			effort: e.benchmark_reasoning_effort,
			metadata_changed: plan.englishMeta !== undefined,
		};
	});
	console.log(
		JSON.stringify(
			{
				mode: apply ? "apply" : "preview",
				api,
				version: AA_V5_VERSION,
				calibration,
				models: summary,
			},
			null,
			2,
		),
	);
	if (!apply) return;
	await writeFile(
		snapshot,
		JSON.stringify(
			{ api, captured_at: new Date().toISOString(), bits, source, calibration },
			null,
			2,
		),
		{ flag: "wx", mode: 0o600 },
	);
	const journal: { id: string; slug: string }[] = [];
	const journalPath = `${snapshot}.applied.json`;
	await writeFile(journalPath, "[]", { flag: "wx", mode: 0o600 });
	const writtenModels = new Map<string, string>();
	for (const plan of plans) {
		const current = await freshBit(plan.bit.id);
		// Earlier writes may update a shared source evaluation, including a remapped artifact's old source.
		assertBitUnchanged(
			plan.bit,
			current,
			!writtenModels.has(plan.bit.model_slug ?? plan.slug),
		);
		const modelJson = canonicalJson(plan.model);
		const written = writtenModels.get(plan.slug);
		if (written && written !== modelJson)
			throw new Error(`Conflicting planned evaluations for ${plan.slug}`);
		if (!written) {
			await request(
				`admin/models/${encodeURIComponent(plan.slug)}`,
				"PUT",
				plan.model,
			);
			writtenModels.set(plan.slug, modelJson);
		}
		const saved = savedBitFromEvents(
			await (
				await request(`admin/bit/${current.id}`, "PUT", {
					...current,
					model_slug: plan.slug,
					parameters: plan.parameters,
				})
			).text(),
		);
		if (saved.id !== current.id || saved.model_slug !== plan.slug)
			throw new Error("Admin response has an unexpected Bit identity");
		// Use the latest copy so unrelated metadata edits made during the refresh survive.
		const metadataCurrent = await freshBit(current.id);
		assertBitUnchanged(
			{ ...current, model_slug: plan.slug, parameters: plan.parameters },
			metadataCurrent,
			false,
		);
		const englishMeta = refreshEnglishMetadata(
			metadataCurrent.meta.en,
			record(plan.model.evaluations),
		);
		if (englishMeta) {
			await request(`admin/bit/${current.id}/en`, "PUT", {
				...englishMeta,
				updated_at: {
					secs_since_epoch: Math.floor(Date.now() / 1000),
					nanos_since_epoch: 0,
				},
			});
		}
		journal.push({ id: current.id, slug: plan.slug });
		await writeFile(journalPath, JSON.stringify(journal, null, 2), {
			mode: 0o600,
		});
		console.log(
			`Updated ${journal.length}/${plans.length}: ${summary.find((row) => row.id === current.id)?.name}`,
		);
	}
	console.log(
		`Updated ${plans.length} language-model entries on AA v${AA_V5_VERSION}; measured ${summary.filter((p) => p.status === "measured").length}, estimated ${summary.filter((p) => p.status === "estimated").length}. Snapshot: ${snapshot}`,
	);
}

if (import.meta.main)
	main().catch((error) => {
		console.error(error instanceof Error ? error.message : "AA refresh failed");
		process.exitCode = 1;
	});
