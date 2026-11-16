import { describe, expect, test } from "bun:test";
import type { OpenRouterPricingSnapshot } from "./catalog-openrouter-pricing";
import type { CatalogPreviewEntry } from "./catalog-preview-types";
import {
	type CareBit,
	assertCareUnchanged,
	assertFreshPricing,
	planCatalogCare,
	stableCatalogValue,
} from "./update-catalog-care";

const snapshot: OpenRouterPricingSnapshot = {
	fetched_at: "2026-11-16T12:00:00Z",
	source_url: "https://openrouter.ai/api/v1/models?output_modalities=all",
	data: [],
	sources: {},
};
const bit: CareBit = {
	id: "voice",
	type: "Tts",
	parameters: {
		pricing: { request_micro_usd: 123 },
		provider: {
			provider_name: "hosted:openrouter",
			model_id: "publisher/voice",
		},
		tiers: ["pro"],
	},
	meta: {
		en: {
			name: "Voice",
			description: "A voice model.",
			long_description: "Owner note.\n\nBenchmark evidence.",
			preview_media: [],
			tags: ["speech"],
			website: "https://example.com/voice",
		},
		de: { name: "Stimme" },
	},
	model_slug: "voice-score",
	model_evaluation: { evaluations: { quality_estimate: { score: 70 } } },
	hash: "immutable-artifact-hash",
};
const preview: CatalogPreviewEntry = {
	bit_ids: [bit.id],
	model_id: "publisher/voice",
	media: [
		{
			url: "https://example.com/voice.wav",
			kind: "audio",
			source_url: "https://example.com/voice",
			description: "Publisher speech sample.",
		},
	],
};

describe("combined catalog care publication", () => {
	test("preserves runtime tariffs, scores, languages and owner metadata while composing references and samples", () => {
		const original = structuredClone(bit);
		const [plan] = planCatalogCare([bit], snapshot, [], [preview]);
		expect(plan.parametersChanged).toBe(true);
		expect(plan.metadataChanged).toBe(true);
		expect(plan.after.parameters?.pricing).toEqual(bit.parameters?.pricing);
		expect(plan.after.parameters?.provider).toEqual(bit.parameters?.provider);
		expect(plan.after.parameters?.tiers).toEqual(["pro"]);
		expect(plan.after.model_evaluation).toEqual(bit.model_evaluation);
		expect(plan.after.model_slug).toBe(bit.model_slug);
		expect(plan.after.hash).toBe(bit.hash);
		expect(plan.after.meta.de).toEqual(bit.meta.de);
		expect(plan.after.meta.en.website).toBe(bit.meta.en.website);
		expect(plan.after.meta.en.long_description).toStartWith(
			String(bit.meta.en.long_description),
		);
		expect(plan.after.meta.en.preview_media).toEqual([preview.media[0].url]);
		expect(bit).toEqual(original);
		expect(
			planCatalogCare([plan.after], snapshot, [], [preview])[0].changed,
		).toBe(false);
	});
	test("creates required English metadata for a known unnamed embedding without changing the model", () => {
		const missing: CareBit = {
			id: "cis3ujaqj8yx0fpfgoye1prn",
			type: "Embedding",
			repository:
				"https://huggingface.co/canavar/clip-ViT-B-32-multilingual-v1-ONNX/resolve/main/model.onnx?download=true",
			file_name: "model.onnx",
			parameters: {
				provider: { provider_name: "Local", model_id: null },
				vector_length: 512,
			},
			meta: {},
		};
		const [plan] = planCatalogCare([missing], snapshot, [], []);
		expect(plan.metadataChanged).toBe(true);
		expect(plan.parametersChanged).toBe(false);
		expect(plan.after.meta.en.name).toBeString();
		expect(plan.after.meta.en.description).toBeString();
		expect(plan.after.meta.en.tags).toEqual([]);
		expect(plan.after.meta.en.preview_media).toEqual([]);
		expect(plan.after.meta.en.created_at).toEqual({
			secs_since_epoch: 1794830400,
			nanos_since_epoch: 0,
		});
		expect(planCatalogCare([plan.after], snapshot, [], [])[0].changed).toBe(
			false,
		);
	});
	test("does not mutate support assets", () => {
		const file: CareBit = {
			id: "tokenizer",
			type: "Tokenizer",
			meta: {},
			parameters: null,
		};
		expect(planCatalogCare([file], snapshot, [], [])[0]).toEqual({
			before: file,
			after: file,
			parametersChanged: false,
			metadataChanged: false,
			changed: false,
		});
	});
	test("rejects concurrent edits but accepts equivalent JSON field ordering", () => {
		expect(() => assertCareUnchanged(bit, structuredClone(bit))).not.toThrow();
		expect(stableCatalogValue({ a: 1, b: [2, 3] })).toBe(
			stableCatalogValue({ b: [2, 3], a: 1 }),
		);
		for (const changed of [
			{ ...bit, hash: "new" },
			{
				...bit,
				meta: {
					...bit.meta,
					en: { ...bit.meta.en, description: "New owner copy" },
				},
			},
			{ ...bit, parameters: { ...bit.parameters, tiers: ["premium"] } },
		])
			expect(() => assertCareUnchanged(bit, changed)).toThrow(
				"changed after planning",
			);
	});
	test("rejects stale, future, invalid and non-OpenRouter source snapshots before publication", () => {
		const now = Date.parse(snapshot.fetched_at);
		expect(() => assertFreshPricing(snapshot, now)).not.toThrow();
		expect(() =>
			assertFreshPricing(snapshot, now + 24 * 60 * 60 * 1000 + 1),
		).toThrow("24 hours");
		expect(() => assertFreshPricing(snapshot, now - 60_001)).toThrow(
			"24 hours",
		);
		expect(() =>
			assertFreshPricing({ ...snapshot, fetched_at: "unknown" }, now),
		).toThrow("24 hours");
		expect(() =>
			assertFreshPricing(
				{ ...snapshot, source_url: "https://example.com/api/v1/models" },
				now,
			),
		).toThrow("OpenRouter models API");
	});
});
