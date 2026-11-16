#!/usr/bin/env bun
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { config } from "dotenv";
import {
	CATALOG_PREVIEWS,
	type PreviewBit,
	planPreviewUpdates,
	verifyPreviewAsset,
} from "./catalog-preview";
import type { CatalogPreviewAsset } from "./catalog-preview-types";

interface CatalogBit extends PreviewBit {
	model_slug?: string | null;
	model_evaluation?: { evaluations?: Record<string, unknown> } | null;
}
const QUALITY_TYPES = new Set([
	"Tts",
	"Stt",
	"Embedding",
	"ImageEmbedding",
	"ImageGeneration",
	"VideoGeneration",
	"ObjectDetection",
	"Forecasting",
	"SystemOne",
]);
const OUTPUT_TYPES = new Set(["Tts", "ImageGeneration", "VideoGeneration"]);

export function qualityCoverage(bits: CatalogBit[]) {
	const categories: Record<string, { total: number; rated: number }> = {};
	const missing: string[] = [];
	for (const bit of bits) {
		if (!QUALITY_TYPES.has(bit.type)) continue;
		categories[bit.type] ??= { total: 0, rated: 0 };
		const category = categories[bit.type];
		category.total++;
		const quality = bit.model_evaluation?.evaluations?.quality_estimate as
			| Record<string, unknown>
			| undefined;
		const numeric = (value: unknown): value is number =>
			typeof value === "number" &&
			Number.isFinite(value) &&
			value >= 0 &&
			value <= 100;
		const valid =
			quality &&
			quality.category === bit.type &&
			numeric(quality.score) &&
			numeric(quality.low) &&
			numeric(quality.high) &&
			quality.low <= quality.score &&
			quality.score <= quality.high &&
			["low", "medium"].includes(String(quality.confidence)) &&
			[quality.criteria, quality.reasoning].every(
				(value) => typeof value === "string" && value.trim().length > 0,
			) &&
			typeof quality.updated_at === "string" &&
			Number.isFinite(Date.parse(quality.updated_at)) &&
			Array.isArray(quality.sources) &&
			quality.sources.length > 0 &&
			quality.sources.every((value) => {
				try {
					const url = new URL(String(value));
					return url.protocol === "https:" && !url.username && !url.password;
				} catch {
					return false;
				}
			});
		if (valid) category.rated++;
		else missing.push(bit.id);
	}
	return { categories, missing };
}

export async function verifyAllAssets(assets: CatalogPreviewAsset[]) {
	const unique = [
		...new Map(assets.map((asset) => [asset.url, asset])).values(),
	];
	const results: Awaited<ReturnType<typeof verifyPreviewAsset>>[] = [];
	let next = 0;
	const settled = await Promise.allSettled(
		Array.from({ length: Math.min(6, unique.length) }, async () => {
			while (next < unique.length) {
				const asset = unique[next++];
				results.push(await verifyPreviewAsset(asset));
			}
		}),
	);
	const errors = settled.filter((result) => result.status === "rejected");
	if (errors.length)
		throw new AggregateError(
			errors.map((result) => (result as PromiseRejectedResult).reason),
			"Some preview assets failed verification; no catalog records were changed",
		);
	return results;
}

export async function main() {
	if (process.argv.includes("--help")) {
		console.log(
			"Usage: bun tools/model-eval/update-catalog-media.ts [--apply --snapshot=/tmp/media-before.json]\nDefaults to a read-only preview. Audits all specialist quality scores and verifies actual preview bytes before writing English metadata. FLOW_LIKE_BASE_URL defaults to https://api.flow-like.com; writes require FLOW_LIKE_PAT.",
		);
		return;
	}
	config({ path: resolve(import.meta.dir, "../../.env"), quiet: true });
	const apply = process.argv.includes("--apply");
	if (apply && process.argv.includes("--dry-run"))
		throw new Error("--apply and --dry-run cannot be combined");
	const snapshot =
		process.argv
			.find((arg) => arg.startsWith("--snapshot="))
			?.slice("--snapshot=".length) ?? "";
	if (apply && !snapshot)
		throw new Error("Provide --snapshot before applying preview metadata");
	const token = process.env.FLOW_LIKE_PAT;
	if (apply && !token) throw new Error("FLOW_LIKE_PAT is required");
	const base = new URL(
		process.env.FLOW_LIKE_BASE_URL ?? "https://api.flow-like.com",
	);
	if (base.protocol !== "https:" || base.username || base.password)
		throw new Error("Catalog base must use HTTPS without URL credentials");
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
	const bits: CatalogBit[] = [];
	for (let offset = 0; ; offset += 100) {
		const language = `preview-audit-${Date.now()}`;
		const page = (await (
			await request(`bit?language=${language}`, "POST", { limit: 100, offset })
		).json()) as CatalogBit[];
		if (!Array.isArray(page)) throw new Error("Invalid catalog response");
		for (const bit of page) delete bit.meta[language];
		bits.push(...page);
		if (page.length < 100) break;
		if (offset >= 10_000)
			throw new Error("Catalog pagination exceeded its safety bound");
	}
	const coverage = qualityCoverage(bits);
	if (coverage.missing.length)
		throw new Error(
			`Specialist model scores are missing or invalid: ${coverage.missing.join(", ")}. Update their category ratings before publishing previews.`,
		);
	const plans = planPreviewUpdates(bits);
	const plannedIds = new Set(plans.map((plan) => plan.bit.id));
	const uncovered = bits
		.filter((bit) => OUTPUT_TYPES.has(bit.type) && !plannedIds.has(bit.id))
		.map((bit) => ({ id: bit.id, name: bit.meta.en?.name, type: bit.type }));
	const verified = await verifyAllAssets(
		CATALOG_PREVIEWS.flatMap((entry) => entry.media),
	);
	console.log(
		JSON.stringify(
			{
				mode: apply ? "apply" : "preview",
				api,
				quality_scores: coverage.categories,
				preview_models: plans.length,
				metadata_updates: plans.filter((plan) => plan.changed).length,
				verified_assets: verified.length,
				uncovered,
				previews: plans.map((plan) => ({
					id: plan.bit.id,
					name: plan.bit.meta.en?.name,
					type: plan.bit.type,
					changed: plan.changed,
					media: plan.entry.media,
				})),
			},
			null,
			2,
		),
	);
	if (!apply) return;
	if (uncovered.length)
		throw new Error(
			"Some output models have no curated previews; review the uncovered list before applying",
		);
	await writeFile(
		snapshot,
		JSON.stringify(
			{
				api,
				captured_at: new Date().toISOString(),
				bits,
				registry: CATALOG_PREVIEWS,
				verified_assets: verified,
				score_coverage: coverage,
			},
			null,
			2,
		),
		{ flag: "wx", mode: 0o600 },
	);
	const journalPath = `${snapshot}.applied.json`;
	const journal: { id: string; preview_media: unknown }[] = [];
	await writeFile(journalPath, "[]", { flag: "wx", mode: 0o600 });
	for (const plan of plans.filter((plan) => plan.changed)) {
		const language = `preview-write-${Date.now()}`;
		const current = (await (
			await request(`bit/${plan.bit.id}?language=${language}`)
		).json()) as CatalogBit;
		delete current.meta[language];
		// Recheck the exact model, and merge with current media and owner-authored copy.
		const fresh = planPreviewUpdates(
			[current],
			[{ ...plan.entry, bit_ids: [current.id] }],
		)[0];
		if (current.id !== plan.bit.id)
			throw new Error("Unexpected Bit identity from catalog read");
		if (!fresh.changed) continue;
		await request(`admin/bit/${current.id}/en`, "PUT", {
			...fresh.meta,
			updated_at: {
				secs_since_epoch: Math.floor(Date.now() / 1000),
				nanos_since_epoch: 0,
			},
		});
		journal.push({ id: current.id, preview_media: fresh.meta.preview_media });
		await writeFile(journalPath, JSON.stringify(journal, null, 2), {
			mode: 0o600,
		});
		console.log(
			`Updated preview metadata ${journal.length}/${plans.filter((plan) => plan.changed).length}: ${current.meta.en.name}`,
		);
	}
	console.log(
		`Published previews for ${journal.length} models. All ${Object.values(coverage.categories).reduce((total, category) => total + category.rated, 0)} specialist models have category quality scores. Snapshot: ${snapshot}`,
	);
}

if (import.meta.main)
	main().catch((error) => {
		if (error instanceof AggregateError)
			for (const cause of error.errors)
				console.error(cause instanceof Error ? cause.message : String(cause));
		console.error(
			error instanceof Error ? error.message : "Catalog preview update failed",
		);
		process.exitCode = 1;
	});
