#!/usr/bin/env bun
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { config } from "dotenv";
import { refreshBenchmarkDescription } from "./aa-v5-metadata";
import {
	type CatalogQualityBit,
	type CatalogQualityRating,
	findCatalogQualityRating,
} from "./catalog-quality";
import { QWEN4_MAX } from "./qwen4-max";

export interface CatalogBitRow extends CatalogQualityBit {
	id: string;
	modelSlug: string | null;
}

interface ApiBit extends CatalogQualityBit {
	id: string;
	model_slug?: string | null;
	model_evaluation?: Record<string, unknown> | null;
	meta: Record<string, unknown>;
	[key: string]: unknown;
}

const MODEL_TYPES = [
	"Llm",
	"Vlm",
	"Tts",
	"Stt",
	"Embedding",
	"ImageEmbedding",
	"ImageGeneration",
	"VideoGeneration",
	"ObjectDetection",
	"Forecasting",
	"SystemOne",
];
const QUALITY_TYPES = new Set(
	MODEL_TYPES.slice(2).map((type) => type.toLowerCase()),
);

export function planQualityUpdates(bits: CatalogBitRow[]) {
	const updates: { bit: CatalogBitRow; rating: CatalogQualityRating }[] = [];
	const unmatched: CatalogBitRow[] = [];
	for (const bit of bits) {
		if (!QUALITY_TYPES.has(bit.type.replaceAll("_", "").toLowerCase()))
			continue;
		const rating = findCatalogQualityRating(bit);
		if (!rating) {
			unmatched.push(bit);
			continue;
		}
		if (bit.modelSlug && bit.modelSlug !== rating.slug) {
			throw new Error(
				`Bit ${bit.id} already links to ${bit.modelSlug}; review its evaluation before replacing it.`,
			);
		}
		updates.push({ bit, rating });
	}
	return { updates, unmatched };
}

export function findQwenBit(bits: CatalogBitRow[]): CatalogBitRow | undefined {
	const matches = bits.filter((bit) => {
		const provider = bit.parameters?.provider as
			| Record<string, unknown>
			| undefined;
		return (
			provider?.provider_name === "hosted:openrouter" &&
			provider.model_id === "qwen/qwen4-max"
		);
	});
	if (matches.length > 1)
		throw new Error(
			"Multiple hosted Qwen4 Max bits exist; choose the intended record before updating.",
		);
	return matches[0];
}

/** Admin Bit writes stream progress; HTTP 200 alone does not confirm a saved Bit. */
export function savedBitFromEvents(events: string): ApiBit {
	let saved: ApiBit | undefined;
	for (const block of events.replaceAll("\r\n", "\n").split("\n\n")) {
		const lines = block.split("\n");
		const event = lines
			.find((line) => line.startsWith("event:"))
			?.slice(6)
			.trim();
		const data = lines
			.filter((line) => line.startsWith("data:"))
			.map((line) => line.slice(5).trimStart())
			.join("\n");
		if (event === "error")
			throw new Error("The admin API reported a Bit write failure.");
		if (event === "done") {
			const bit = JSON.parse(data) as ApiBit;
			if (typeof bit.id !== "string" || !bit.id)
				throw new Error("Invalid saved Bit response");
			saved = bit;
		}
	}
	if (!saved) throw new Error("The admin API did not confirm the Bit write.");
	return saved;
}

/** The admin model endpoint accepts a date; public reads return RFC3339. */
export function catalogReleaseDate(value: unknown): string | null {
	return typeof value === "string" && /^\d{4}-\d{2}-\d{2}(?:T|$)/.test(value)
		? value.slice(0, 10)
		: null;
}

/** A seed re-run must not replace results maintained by the versioned refresh. */
export function qwenBenchmarkState(existing?: {
	model_evaluation?: Record<string, unknown> | null;
	parameters?: Record<string, unknown> | null;
}) {
	const evaluations = existing?.model_evaluation?.evaluations as
		| Record<string, unknown>
		| undefined;
	const version = String(
		evaluations?.artificial_analysis_intelligence_index_version ?? "",
	).replace(/^v/, "");
	const parameters = { ...QWEN4_MAX.parameters } as Record<string, unknown>;
	if (Number.parseInt(version, 10) >= 5) {
		for (const [key, value] of Object.entries(existing?.parameters ?? {})) {
			if (
				key.startsWith("model_classification") ||
				key.startsWith("classification_") ||
				key.startsWith("legacy_") ||
				key.startsWith("benchmark_")
			)
				parameters[key] = value;
		}
		return { evaluations, parameters };
	}
	return {
		evaluations: { ...evaluations, ...QWEN4_MAX.evaluations },
		parameters,
	};
}

async function main() {
	if (process.argv.includes("--help")) {
		console.log(
			"Usage: bun tools/model-eval/update-catalog.ts [--apply --snapshot=/tmp/catalog-before.json] [--qwen-only]\nDefaults to a read-only preview of the deployed API. FLOW_LIKE_BASE_URL defaults to https://api.flow-like.com. Writes use FLOW_LIKE_PAT from the environment or repository .env.",
		);
		return;
	}
	config({ path: resolve(import.meta.dir, "../../.env"), quiet: true });
	const apply = process.argv.includes("--apply");
	const qwenOnly = process.argv.includes("--qwen-only");
	const snapshot =
		process.argv
			.find((arg) => arg.startsWith("--snapshot="))
			?.slice("--snapshot=".length) ?? "";
	if (apply && !snapshot)
		throw new Error(
			"Provide --snapshot to save the affected records before applying.",
		);
	const token = process.env.FLOW_LIKE_PAT;
	if (apply && !token)
		throw new Error(
			"FLOW_LIKE_PAT is required to update the deployed catalog.",
		);
	const base = new URL(
		process.env.FLOW_LIKE_BASE_URL ?? "https://api.flow-like.com",
	);
	if (base.protocol !== "https:" || base.username || base.password)
		throw new Error(
			"FLOW_LIKE_BASE_URL must be an HTTPS URL without credentials.",
		);
	const apiBase = `${base.origin}${base.pathname.replace(/\/$/, "").replace(/\/api\/v1$/, "")}/api/v1`;
	async function request(path: string, method = "GET", body?: unknown) {
		const response = await fetch(`${apiBase}/${path}`, {
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
	const bits: ApiBit[] = [];
	for (let offset = 0; ; offset += 100) {
		const page = (await (
			await request("bit", "POST", {
				bit_types: MODEL_TYPES,
				limit: 100,
				offset,
			})
		).json()) as ApiBit[];
		if (!Array.isArray(page)) throw new Error("Invalid catalog response");
		bits.push(...page);
		if (page.length < 100) break;
		if (offset >= 10_000)
			throw new Error("Catalog pagination exceeded its safety bound");
	}
	const rows = bits.map((bit) => ({
		...bit,
		modelSlug: bit.model_slug ?? null,
	}));
	const { updates, unmatched } = planQualityUpdates(qwenOnly ? [] : rows);
	const existingQwen = findQwenBit(rows);
	console.log(
		JSON.stringify(
			{
				mode: apply ? "apply" : "preview",
				api: apiBase,
				qwen: {
					action: existingQwen ? "update" : "create",
					score: QWEN4_MAX.evaluations.artificial_analysis_intelligence_index,
					type: "Vlm",
				},
				quality: updates.map(({ bit, rating }) => ({
					id: bit.id,
					name: rating.name,
					category: rating.quality_estimate.category,
					score: rating.quality_estimate.score,
				})),
				unmatched: unmatched.map((bit) => ({
					id: bit.id,
					type: bit.type,
					repository: bit.repository,
				})),
			},
			null,
			2,
		),
	);
	if (!apply) return;
	if (unmatched.length)
		throw new Error(
			"Unrated model identities remain. Add explicit ratings before applying.",
		);
	const affected = new Set([
		...updates.map(({ bit }) => bit.id),
		...(existingQwen ? [existingQwen.id] : []),
	]);
	await writeFile(
		snapshot,
		JSON.stringify(
			{
				api: apiBase,
				captured_at: new Date().toISOString(),
				bits: bits.filter((bit) => affected.has(bit.id)),
			},
			null,
			2,
		),
		{ flag: "wx", mode: 0o600 },
	);
	const completed: { id: string; slug: string }[] = [];
	await writeFile(`${snapshot}.applied.json`, "[]", {
		flag: "wx",
		mode: 0o600,
	});
	async function record(bit: ApiBit) {
		completed.push({ id: bit.id, slug: bit.model_slug ?? "" });
		await writeFile(
			`${snapshot}.applied.json`,
			JSON.stringify(completed, null, 2),
			{ mode: 0o600 },
		);
	}
	async function saveBit(bit: ApiBit) {
		const saved = savedBitFromEvents(
			await (
				await request(`admin/bit/${encodeURIComponent(bit.id)}`, "PUT", bit)
			).text(),
		);
		if (saved.model_slug !== bit.model_slug)
			throw new Error(
				`Saved Bit ${saved.id} has an unexpected model association`,
			);
		return saved;
	}

	const qwen = QWEN4_MAX;
	const previousQwen = existingQwen
		? bits.find((bit) => bit.id === existingQwen.id)
		: undefined;
	const benchmarkState = qwenBenchmarkState(previousQwen);
	await request(`admin/models/${qwen.slug}`, "PUT", {
		...previousQwen?.model_evaluation,
		name: qwen.name,
		release_date: qwen.releaseDate,
		creator_name: qwen.creatorName,
		creator_slug: qwen.creatorSlug,
		evaluations: benchmarkState.evaluations,
		pricing: qwen.pricing,
	});
	const now = new Date().toISOString();
	const savedQwen = await saveBit({
		...(previousQwen ?? {
			id: "qwen4-max-openrouter",
			hash: "",
			dependency_tree_hash: "",
			dependencies: [],
			download_link: null,
			file_name: null,
			size: 0,
			version: "0.0.1",
			created: now,
			updated: now,
		}),
		type: "Vlm",
		meta: previousQwen?.meta ?? {},
		authors: qwen.authors,
		hub: qwen.hub,
		repository: qwen.repository,
		license: qwen.license,
		parameters: {
			...(previousQwen?.parameters ?? {}),
			...benchmarkState.parameters,
		},
		model_slug: qwen.slug,
	});
	await record(savedQwen);
	const time = {
		secs_since_epoch: Math.floor(Date.now() / 1000),
		nanos_since_epoch: 0,
	};
	const previousEnglishMeta = previousQwen?.meta.en as
		| Record<string, unknown>
		| undefined;
	await request(`admin/bit/${savedQwen.id}/en`, "PUT", {
		...previousEnglishMeta,
		name: qwen.meta.name,
		description: qwen.meta.description,
		long_description: refreshBenchmarkDescription(
			qwen.meta.longDescription,
			benchmarkState.evaluations ?? {},
		),
		tags: qwen.meta.tags,
		use_case: qwen.meta.useCase,
		website: qwen.meta.website,
		docs_url: qwen.meta.docsUrl,
		icon: qwen.meta.icon,
		preview_media: previousEnglishMeta?.preview_media ?? [],
		created_at: time,
		updated_at: time,
	});
	console.log(`Saved Qwen4 Max as ${savedQwen.id}`);
	for (const { bit, rating } of updates) {
		// Re-read each Bit so the update preserves current runtime configuration.
		const current = (await (
			await request(`bit/${bit.id}?language=en`)
		).json()) as ApiBit;
		if (
			findCatalogQualityRating(current)?.slug !== rating.slug ||
			(current.model_slug && current.model_slug !== rating.slug)
		)
			throw new Error(
				`Bit ${bit.id} changed since the preview; stopped before replacing its evaluation.`,
			);
		await request(`admin/models/${encodeURIComponent(rating.slug)}`, "PUT", {
			...current.model_evaluation,
			release_date: catalogReleaseDate(current.model_evaluation?.release_date),
			name: rating.name,
			creator_name: rating.creator_name,
			creator_slug: rating.creator_slug,
			evaluations: {
				...((current.model_evaluation?.evaluations as Record<
					string,
					unknown
				>) ?? {}),
				quality_estimate: rating.quality_estimate,
				...(rating.reported_benchmarks
					? { reported_benchmarks: rating.reported_benchmarks }
					: {}),
			},
		});
		await record(await saveBit({ ...current, model_slug: rating.slug }));
		if (completed.length % 20 === 0)
			console.log(
				`Saved ${completed.length - 1}/${updates.length} quality ratings`,
			);
	}
	console.log(
		`Applied ${updates.length} quality ratings and Qwen4 Max through ${apiBase}. Snapshot: ${snapshot}.`,
	);
}

if (import.meta.main) {
	main().catch((error) => {
		console.error(
			error instanceof Error ? error.message : "Catalog update failed",
		);
		process.exitCode = 1;
	});
}
