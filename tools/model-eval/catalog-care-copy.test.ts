import { describe, expect, test } from "bun:test";
import { type CareCopyBit, getCatalogCopyPatch } from "./catalog-care-copy";

function embedding(overrides: Partial<CareCopyBit> = {}): CareCopyBit {
	return {
		id: "gccb4h3bj8jdxxis2vcjuxuk",
		type: "Embedding",
		repository:
			"https://huggingface.co/intfloat/multilingual-e5-large-instruct",
		file_name: "model.onnx",
		parameters: { provider: { model_id: null, provider_name: "Local" } },
		meta: {
			en: {
				name: "Multilingual - Instruct",
				description:
					"The best multilingual model we have. Only available for paragraphs up to 512 tokens.",
				long_description:
					"Multilingual - E5 - Large Instruct is the best retrieval model you can use for your case in multilingual context.",
			},
		},
		...overrides,
	};
}

describe("catalog copy care", () => {
	test("ignores support files and unknown model identities", () => {
		expect(getCatalogCopyPatch(embedding({ type: "File" }))).toBeNull();
		expect(getCatalogCopyPatch(embedding({ id: "unreviewed" }))).toBeNull();
	});

	test("refuses a reviewed Bit that now refers to another model", () => {
		expect(() =>
			getCatalogCopyPatch(
				embedding({ repository: "https://example.com/changed" }),
			),
		).toThrow("identity changed");
	});

	test("replaces unsupported introductory claims without changing later evidence", () => {
		const bit = embedding();
		const meta = (
			bit.meta as {
				en: { name: string; description: string; long_description: string };
			}
		).en;
		const evidence =
			"\n\nQuality estimate: 80, low confidence. Source: https://example.com/evidence\n\nPreview samples and sources:\n1. Publisher example. Source: https://example.com/sample\n\nOwner note: keep this.";
		meta.long_description += evidence;
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.name).toBe("Multilingual E5 Large Instruct");
		expect(patch?.long_description).toEndWith(evidence);
		expect(patch?.long_description).toContain(
			"Rebuild the index when changing embedding models",
		);
		expect(patch?.long_description).not.toContain("best retrieval model");
		expect(patch?.sources).toContain(bit.repository as string);
	});

	test("keeps a newly edited introduction and is idempotent", () => {
		const bit = embedding();
		const current = {
			name: "Multilingual - Instruct",
			description: "Existing description",
			long_description:
				"New owner-written introduction.\n\nPreview samples and sources:\n1. Keep me. Source: https://example.com",
		};
		bit.meta = { en: current };
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.long_description).toBe(current.long_description);
		bit.meta = { en: { ...current, ...patch } };
		expect(getCatalogCopyPatch(bit)).toBeNull();
	});

	test("removes the provider suffix while preserving hosted voice and preview details", () => {
		const longDescription =
			"Default voice: longanlingxin.\n\nPreview samples and sources:\n1. Actual sample. Source: https://openrouter.ai/benchmarks/media/speech/plain-sentence";
		const bit: CareCopyBit = {
			id: "or0024b06cd282b54e9ff14d",
			type: "Tts",
			file_name: null,
			repository: "https://openrouter.ai/qwen/qwen-audio-3.0-tts-plus",
			parameters: { provider: { model_id: "qwen/qwen-audio-3.0-tts-plus" } },
			meta: {
				en: {
					name: "Qwen-Audio-3.0-TTS Plus · OpenRouter",
					description: "Speak replies. Hosted through OpenRouter.",
					long_description: longDescription,
				},
			},
		};
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.name).toBe("Qwen-Audio-3.0-TTS Plus");
		expect(patch?.long_description).toBe(longDescription);
	});

	test("fills absent embedding metadata without changing model fields", () => {
		const bit: CareCopyBit = {
			id: "dztj51d8oexo6skxiy22tn28",
			type: "Embedding",
			file_name: "model.onnx",
			repository:
				"https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main/onnx/model.onnx",
			parameters: { provider: { model_id: null } },
			meta: {},
		};
		const before = JSON.stringify(bit);
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.name).toBe("Nomic Embed Text v1.5");
		expect(patch?.long_description).toContain("768-dimensional vectors");
		expect(JSON.stringify(bit)).toBe(before);
	});

	test("removes duplicate provider amounts while preserving limits, scores and reservation notes", () => {
		const benchmark =
			"Artificial Analysis Intelligence Index v5.0.1: 42.89 for Qwen4 Max. Source: https://artificialanalysis.ai/models";
		const pricing =
			"The catalog uses OpenRouter's live endpoint limit of 131,072 output tokens and a total context window of 1,000,000 tokens. Standard rates are $1.50 per million input tokens and $4.50 per million output tokens. This is a proprietary hosted model.";
		const reservation =
			"Runtime reservation: $5 is a configured spending ceiling, not the provider's unit price.";
		const bit: CareCopyBit = {
			id: "hfxqyodgq5w607hssl9u0a57",
			type: "Vlm",
			file_name: null,
			repository: "https://openrouter.ai/qwen/qwen4-max",
			parameters: { provider: { model_id: "qwen/qwen4-max" } },
			meta: {
				en: {
					name: "Qwen4 Max",
					description: "Work with text and images.",
					long_description: [benchmark, pricing, reservation].join("\n\n"),
				},
			},
		};
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.long_description).toContain(benchmark);
		expect(patch?.long_description).toContain(reservation);
		expect(patch?.long_description).toContain("131,072 output tokens");
		expect(patch?.long_description).toContain("1,000,000 tokens");
		expect(patch?.long_description).not.toContain("$1.50");
		expect(patch?.long_description).toContain(
			"Provider rate references and Flow-Like usage charges are listed separately",
		);
		bit.meta = { en: patch };
		expect(getCatalogCopyPatch(bit)).toBeNull();
	});

	test("states verified primary and fallback models without changing the benchmark reference", () => {
		const benchmark =
			"Artificial Analysis Intelligence Index v5.0.1: 54.61 for GPT-6.1 Sol (Max).";
		const bit: CareCopyBit = {
			id: "dt7121ak3swi6l45s3ei0g3e",
			type: "Vlm",
			file_name: null,
			repository: "https://flow-like.com/models",
			parameters: { provider: { model_id: "@preset/gpt-5" } },
			meta: {
				en: {
					name: "GPT",
					description: "Owner description",
					long_description: benchmark,
				},
			},
		};
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.name).toBe("GPT");
		expect(patch?.long_description).toContain(benchmark);
		expect(patch?.long_description).toContain("primary model [GPT-6.1 Sol]");
		expect(patch?.long_description).toContain(
			"fallbacks in order: [GPT-5.6 Sol]",
		);
		expect(patch?.long_description).toContain(
			"Fallback models can differ in quality, features and price",
		);
		expect(patch?.long_description).not.toContain("workspace_id");
		bit.meta = { en: patch };
		expect(getCatalogCopyPatch(bit)).toBeNull();
	});

	test("discloses the Free preset's paid fallback on the card and in routing details", () => {
		const bit: CareCopyBit = {
			id: "ca6ziza17c8xe6scwlnl42my",
			type: "Vlm",
			file_name: null,
			repository: "https://flow-like.com/models",
			parameters: { provider: { model_id: "@preset/prod-free" } },
			meta: {
				en: {
					name: "Free",
					description: "Free models.",
					long_description: "Preserve this benchmark explanation.",
				},
			},
		};
		const patch = getCatalogCopyPatch(bit);
		expect(patch?.name).toBe("Free Router with Paid Fallback");
		expect(patch?.description).toContain("paid DeepSeek V4.1 Flash");
		expect(patch?.long_description).toStartWith(
			"Preserve this benchmark explanation.",
		);
		expect(patch?.long_description).toContain("deepseek/deepseek-v4.1-flash");
		expect(patch?.long_description).toContain("thinkingmachines/inkling:free");
	});
});
