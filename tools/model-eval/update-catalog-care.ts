#!/usr/bin/env bun
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { config } from "dotenv";
import { getCatalogCopyPatch } from "./catalog-care-copy";
import {
	type OpenRouterPricingSnapshot,
	type PresetPriceTarget,
	planOpenRouterPricing,
} from "./catalog-openrouter-pricing";
import {
	CATALOG_PREVIEWS,
	type PreviewBit,
	planPreviewUpdates,
} from "./catalog-preview";
import type { CatalogPreviewEntry } from "./catalog-preview-types";
import { savedBitFromEvents } from "./update-catalog";
import { qualityCoverage, verifyAllAssets } from "./update-catalog-media";

export interface CareBit extends PreviewBit {
	[key: string]: unknown;
	model_slug?: string | null;
	model_evaluation?: { evaluations?: Record<string, unknown> } | null;
}

export function stableCatalogValue(value: unknown): string {
	if (Array.isArray(value))
		return `[${value.map(stableCatalogValue).join(",")}]`;
	if (value && typeof value === "object")
		return `{${Object.entries(value)
			.filter(([, item]) => item !== undefined)
			.sort(([a], [b]) => a.localeCompare(b))
			.map(
				([key, item]) => `${JSON.stringify(key)}:${stableCatalogValue(item)}`,
			)
			.join(",")}}`;
	return JSON.stringify(value) ?? "null";
}

export function assertFreshPricing(
	snapshot: OpenRouterPricingSnapshot,
	now = Date.now(),
) {
	const age = now - Date.parse(snapshot.fetched_at);
	if (!Number.isFinite(age) || age < -60_000 || age > 24 * 60 * 60 * 1000)
		throw new Error(
			"OpenRouter pricing evidence must be less than 24 hours old",
		);
	const url = new URL(snapshot.source_url);
	if (
		url.origin !== "https://openrouter.ai" ||
		url.pathname !== "/api/v1/models" ||
		url.username ||
		url.password
	)
		throw new Error(
			"Pricing snapshot must come from the OpenRouter models API",
		);
}

/** Compose reviewed copy, provider references and media without changing runtime tariffs or scores. */
export function planCatalogCare(
	bits: CareBit[],
	pricing: OpenRouterPricingSnapshot,
	presets: PresetPriceTarget[] = [],
	previews: CatalogPreviewEntry[] = CATALOG_PREVIEWS,
) {
	const prices = new Map(
		planOpenRouterPricing(bits, pricing, presets).map((plan) => [
			plan.bit_id,
			plan,
		]),
	);
	const prepared: CareBit[] = bits.map((bit) => {
		const copy = getCatalogCopyPatch(bit);
		const price = prices.get(bit.id);
		const next: CareBit = { ...bit, meta: { ...bit.meta } };
		if (copy) {
			const timestamp = {
				secs_since_epoch: Math.floor(Date.parse(pricing.fetched_at) / 1000),
				nanos_since_epoch: 0,
			};
			next.meta.en = {
				...(bit.meta.en ?? {
					tags: [],
					preview_media: [],
					created_at: timestamp,
					updated_at: timestamp,
				}),
				name: copy.name,
				description: copy.description,
				long_description: copy.long_description,
			};
		}
		if (price)
			next.parameters = {
				...bit.parameters,
				pricing_reference: price.pricing_reference,
			};
		return next;
	});
	const media = new Map(
		planPreviewUpdates(prepared, previews).map((plan) => [plan.bit.id, plan]),
	);
	return prepared.map((after, index) => {
		const before = bits[index];
		const preview = media.get(after.id);
		if (preview) after.meta.en = preview.meta;
		const parametersChanged =
			stableCatalogValue(before.parameters) !==
			stableCatalogValue(after.parameters);
		const metadataChanged =
			stableCatalogValue(before.meta.en) !== stableCatalogValue(after.meta.en);
		return {
			before,
			after,
			parametersChanged,
			metadataChanged,
			changed: parametersChanged || metadataChanged,
		};
	});
}

/** Stop on concurrent edits before sending the full Bit replacement required by the API. */
export function assertCareUnchanged(expected: CareBit, current: CareBit) {
	if (stableCatalogValue(expected) !== stableCatalogValue(current))
		throw new Error(
			`Bit ${expected.id} changed after planning; refresh and review before retrying`,
		);
}

export async function main() {
	const argument = (name: string) =>
		process.argv
			.find((arg) => arg.startsWith(`--${name}=`))
			?.slice(name.length + 3);
	if (process.argv.includes("--help")) {
		console.log(
			"Usage: bun tools/model-eval/update-catalog-care.ts --pricing-source=/tmp/openrouter.json [--preset-references=/tmp/presets.json] [--apply --snapshot=/tmp/care-before.json]\nDefaults to a read-only preview. Verifies preview bytes and category ratings, preserves runtime tariffs and benchmark scores, then writes reviewed English copy and provider price references through the catalog API.",
		);
		return;
	}
	config({ path: resolve(import.meta.dir, "../../.env"), quiet: true });
	const apply = process.argv.includes("--apply");
	if (apply && process.argv.includes("--dry-run"))
		throw new Error("--apply and --dry-run cannot be combined");
	const pricingPath = argument("pricing-source");
	const snapshotPath = argument("snapshot");
	const presetPath = argument("preset-references");
	if (!pricingPath)
		throw new Error(
			"Provide --pricing-source with current OpenRouter API evidence",
		);
	if (apply && !snapshotPath)
		throw new Error("Provide --snapshot before applying catalog care");
	const pricing: OpenRouterPricingSnapshot = JSON.parse(
		await readFile(pricingPath, "utf8"),
	);
	const presets: PresetPriceTarget[] = presetPath
		? JSON.parse(await readFile(presetPath, "utf8"))
		: [];
	assertFreshPricing(pricing);
	const token = process.env.FLOW_LIKE_PAT;
	if (apply && !token) throw new Error("FLOW_LIKE_PAT is required");
	const base = new URL(
		process.env.FLOW_LIKE_BASE_URL ?? "https://api.flow-like.com",
	);
	if (base.protocol !== "https:" || base.username || base.password)
		throw new Error("Catalog base requires HTTPS without URL credentials");
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
	async function readBit(id: string) {
		const language = `care-${crypto.randomUUID()}`;
		const bit = (await (
			await request(`bit/${encodeURIComponent(id)}?language=${language}`)
		).json()) as CareBit;
		if (bit.id !== id || !bit.meta)
			throw new Error("Invalid catalog Bit response");
		delete bit.meta[language];
		return bit;
	}
	const bits: CareBit[] = [];
	for (let offset = 0; ; offset += 100) {
		const language = `care-${crypto.randomUUID()}`;
		const page = (await (
			await request(`bit?language=${language}`, "POST", { limit: 100, offset })
		).json()) as CareBit[];
		if (!Array.isArray(page)) throw new Error("Invalid catalog list response");
		for (const bit of page) delete bit.meta[language];
		bits.push(...page);
		if (page.length < 100) break;
		if (offset >= 10_000)
			throw new Error("Catalog pagination exceeded its safety bound");
	}
	const coverage = qualityCoverage(bits);
	if (coverage.missing.length)
		throw new Error(`Invalid category ratings: ${coverage.missing.join(", ")}`);
	const plans = planCatalogCare(bits, pricing, presets);
	const changed = plans.filter((plan) => plan.changed);
	const verified = await verifyAllAssets(
		CATALOG_PREVIEWS.flatMap((entry) => entry.media),
	);
	console.log(
		JSON.stringify(
			{
				mode: apply ? "apply" : "preview",
				bits: bits.length,
				changed: changed.length,
				parameters: changed.filter((plan) => plan.parametersChanged).length,
				metadata: changed.filter((plan) => plan.metadataChanged).length,
				verified_assets: verified.length,
				score_coverage: coverage.categories,
				updates: changed.map(
					({ after, parametersChanged, metadataChanged }) => ({
						id: after.id,
						name: after.meta.en?.name,
						parametersChanged,
						metadataChanged,
						pricing_reference: after.parameters?.pricing_reference,
					}),
				),
			},
			null,
			2,
		),
	);
	if (!apply || !snapshotPath) return;
	await writeFile(
		snapshotPath,
		JSON.stringify(
			{
				captured_at: new Date().toISOString(),
				api,
				bits,
				pricing,
				presets,
				registry: CATALOG_PREVIEWS,
				verified_assets: verified,
				plans,
			},
			null,
			2,
		),
		{ flag: "wx", mode: 0o600 },
	);
	const journalPath = `${snapshotPath}.applied.json`;
	const journal: { id: string; stage: string; at: string }[] = [];
	await writeFile(journalPath, "[]", { flag: "wx", mode: 0o600 });
	async function record(id: string, stage: string) {
		journal.push({ id, stage, at: new Date().toISOString() });
		await writeFile(journalPath, JSON.stringify(journal, null, 2), {
			mode: 0o600,
		});
	}
	for (const [index, plan] of changed.entries()) {
		const current = await readBit(plan.before.id);
		assertCareUnchanged(plan.before, current);
		if (plan.parametersChanged) {
			const saved = savedBitFromEvents(
				await (
					await request(`admin/bit/${current.id}`, "PUT", {
						...current,
						parameters: plan.after.parameters,
					})
				).text(),
			);
			if (saved.id !== current.id)
				throw new Error("Admin API returned a different Bit identity");
			await record(current.id, "parameters");
		}
		if (plan.metadataChanged) {
			if (plan.parametersChanged) {
				const latest = await readBit(current.id);
				if (
					stableCatalogValue(latest.meta) !== stableCatalogValue(current.meta)
				)
					throw new Error(
						`Metadata changed during parameter publication for ${current.id}; refresh before retrying`,
					);
			}
			await request(`admin/bit/${current.id}/en`, "PUT", {
				...plan.after.meta.en,
				updated_at: {
					secs_since_epoch: Math.floor(Date.now() / 1000),
					nanos_since_epoch: 0,
				},
			});
			await record(current.id, "metadata");
		}
		const saved = await readBit(current.id);
		if (
			stableCatalogValue(saved.parameters) !==
			stableCatalogValue(plan.after.parameters)
		)
			throw new Error(`Parameter verification failed for ${current.id}`);
		if (plan.metadataChanged) {
			for (const field of Object.keys(plan.after.meta.en).filter(
				(field) =>
					field !== "updated_at" &&
					(field !== "created_at" || !!current.meta.en),
			))
				if (
					stableCatalogValue(saved.meta.en[field]) !==
					stableCatalogValue(plan.after.meta.en[field])
				)
					throw new Error(
						`Metadata verification failed for ${current.id}: ${field}`,
					);
		}
		for (const field of Object.keys(current).filter(
			(field) => !["updated", "parameters", "meta"].includes(field),
		))
			if (
				stableCatalogValue(saved[field]) !== stableCatalogValue(current[field])
			)
				throw new Error(
					`Preserved Bit field changed for ${current.id}: ${field}`,
				);
		for (const language of Object.keys(current.meta))
			if (
				(language !== "en" || !plan.metadataChanged) &&
				stableCatalogValue(saved.meta[language]) !==
					stableCatalogValue(current.meta[language])
			)
				throw new Error(
					`Preserved metadata changed for ${current.id}: ${language}`,
				);
		await record(current.id, "verified");
		console.log(
			`Verified ${index + 1}/${changed.length}: ${plan.after.meta.en?.name ?? current.id}`,
		);
	}
	console.log(
		`Published and verified catalog care for ${changed.length} Bits. Snapshot: ${snapshotPath}`,
	);
}

if (import.meta.main)
	main().catch((error) => {
		if (error instanceof AggregateError)
			for (const cause of error.errors)
				console.error(cause instanceof Error ? cause.message : String(cause));
		console.error(
			error instanceof Error ? error.message : "Catalog care failed",
		);
		process.exitCode = 1;
	});
