import { describe, expect, test } from "bun:test";
import {
	CATALOG_QUALITY_RATINGS,
	findCatalogQualityRating,
} from "./catalog-quality";

function speech(type: string, modelType: string, modelId?: string) {
	return {
		type,
		parameters: {
			model_type: modelType,
			...(modelId ? { provider: { model_id: modelId } } : {}),
		},
	};
}

describe("catalog quality identities", () => {
	test("distinguishes OLMo checkpoints sharing the same repository", () => {
		const tiny = findCatalogQualityRating(
			speech("STT", "OlmoAsrTinyEn", "allenai/OLMoASR"),
		);
		const large = findCatalogQualityRating(
			speech("Stt", "OlmoAsrLargeEnV2", "allenai/OLMoASR"),
		);
		expect(tiny?.slug).toBe("flow-like-estimate-quality-olmoasr-tiny-en");
		expect(large?.slug).toBe("flow-like-estimate-quality-olmoasr-large-v2-en");
		expect(tiny?.quality_estimate.score).toBeLessThan(
			large?.quality_estimate.score ?? 0,
		);
		expect(
			findCatalogQualityRating({
				type: "STT",
				repository: "https://huggingface.co/allenai/OLMoASR",
			}),
		).toBeNull();
	});

	test("rejects unknown variants rather than inheriting a family's score", () => {
		expect(
			findCatalogQualityRating(
				speech("TTS", "Qwen3Tts", "Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice"),
			),
		).toBeNull();
		expect(
			findCatalogQualityRating(
				speech("STT", "WhisperTiny", "openai/whisper-large-v3-turbo"),
			),
		).toBeNull();
		expect(
			findCatalogQualityRating(
				speech("TTS", "Kokoro", "unreviewed/Kokoro-finetune"),
			),
		).toBeNull();
	});

	test("matches canonical model paths, including download URLs, without host or prefix confusion", () => {
		const input = {
			type: "EMBEDDING",
			repository:
				"https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main/onnx/model.onnx?download=true",
		};
		expect(findCatalogQualityRating(input)?.name).toBe("Nomic Embed Text v1.5");
		expect(
			findCatalogQualityRating({
				...input,
				repository: input.repository.replace(
					"huggingface.co",
					"huggingface.co.example.com",
				),
			}),
		).toBeNull();
		expect(
			findCatalogQualityRating({
				...input,
				repository: input.repository.replace("v1.5/", "v1.5-custom/"),
			}),
		).toBeNull();
		expect(findCatalogQualityRating({ ...input, type: "FILE" })).toBeNull();
	});

	test("accepts explicit runtime types only when no conflicting model identity exists", () => {
		expect(findCatalogQualityRating(speech("TTS", "Kokoro"))?.name).toBe(
			"Kokoro-82M",
		);
		expect(
			findCatalogQualityRating({
				...speech("TTS", "Kokoro"),
				repository: "https://example.org/unknown",
			}),
		).toBeNull();
	});

	test("repository fallback cannot confuse hosted and Hugging Face identities", () => {
		expect(
			findCatalogQualityRating({
				type: "Tts",
				repository: "https://openrouter.ai/elevenlabs/eleven-v4",
			})?.name,
		).toBe("Eleven v4");
		expect(
			findCatalogQualityRating({
				type: "Tts",
				repository: "https://huggingface.co/elevenlabs/eleven-v4",
			}),
		).toBeNull();
		expect(
			findCatalogQualityRating({
				type: "Tts",
				repository: "https://user:secret@openrouter.ai/elevenlabs/eleven-v4",
			}),
		).toBeNull();
	});

	test("free hosting does not lower the same model's output-quality score", () => {
		const paid = findCatalogQualityRating({
			type: "Tts",
			parameters: { provider: { model_id: "fish-audio/s2.1-pro" } },
		});
		const free = findCatalogQualityRating({
			type: "Tts",
			parameters: { provider: { model_id: "fish-audio/s2.1-pro-free:free" } },
		});
		expect(free?.quality_estimate.score).toBe(paid?.quality_estimate.score);
	});

	test("covers each reviewed identity and both database and API type spellings", () => {
		expect(CATALOG_QUALITY_RATINGS.length).toBeGreaterThanOrEqual(82);
		for (const rating of CATALOG_QUALITY_RATINGS) {
			for (const modelId of rating.model_ids) {
				for (const type of [
					rating.quality_estimate.category,
					rating.quality_estimate.category
						.replace("ImageEmbedding", "IMAGE_EMBEDDING")
						.toUpperCase(),
				]) {
					expect(
						findCatalogQualityRating({
							type,
							parameters: {
								model_type: rating.model_type,
								provider: { model_id: modelId },
							},
						})?.slug,
					).toBe(rating.slug);
				}
			}
		}
	});
});

describe("catalog quality evidence", () => {
	test("keeps editorial ratings bounded, sourced and separate from measured intelligence", () => {
		expect(new Set(CATALOG_QUALITY_RATINGS.map((r) => r.slug)).size).toBe(
			CATALOG_QUALITY_RATINGS.length,
		);
		for (const rating of CATALOG_QUALITY_RATINGS) {
			const estimate = rating.quality_estimate;
			expect(estimate.low).toBeGreaterThanOrEqual(0);
			expect(estimate.high).toBeLessThanOrEqual(100);
			expect(estimate.low).toBeLessThan(estimate.score);
			expect(estimate.high).toBeGreaterThan(estimate.score);
			expect(estimate.reasoning).toContain("Editorial estimate");
			expect(estimate.criteria).toContain("within");
			expect(estimate.sources.length).toBeGreaterThan(0);
			for (const source of estimate.sources)
				expect(new URL(source).protocol).toBe("https:");
			expect(Number.isFinite(Date.parse(estimate.updated_at))).toBe(true);
			expect(rating.slug.startsWith("flow-like-estimate-")).toBe(true);
			expect(rating).not.toHaveProperty(
				"artificial_analysis_intelligence_index",
			);
		}
	});

	test("publisher metrics preserve scope, direction and units", () => {
		const asr = findCatalogQualityRating(
			speech("STT", "Qwen3Asr17B", "Qwen/Qwen3-ASR-1.7B"),
		);
		expect(asr?.reported_benchmarks?.[0]).toMatchObject({
			score: 4.9,
			unit: "% WER",
			direction: "lower",
		});
		expect(asr?.reported_benchmarks?.[0].scope).toContain("12-language");
		const vision = findCatalogQualityRating({
			type: "ImageEmbedding",
			repository: "https://huggingface.co/nomic-ai/nomic-embed-vision-v1.5",
		});
		expect(vision?.reported_benchmarks?.[0]).toMatchObject({
			score: 71,
			unit: "% accuracy",
			direction: "higher",
		});
	});
});
