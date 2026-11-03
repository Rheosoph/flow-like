import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	applyGenerationModelPreset,
	createGenerationAssetDrafts,
	defaultGenerationPreset,
	prepareGenerationModelRootBit,
} from "./generation-model-preset";
import {
	createHostedImageBit,
	isHostedImageGeneration,
	updateHostedImageBit,
	updateHostedImageTier,
	validateHostedImageParameters,
} from "./hosted-image-generation";

let id = 0;
function draft(): IBit {
	return {
		id: `image-${++id}`,
		authors: [],
		dependencies: [],
		dependency_tree_hash: "",
		hash: "",
		hub: "",
		created: "",
		updated: "",
		parameters: {},
		type: IBitTypes.ImageGeneration,
		meta: {
			en: {
				name: "Image",
				description: "",
				tags: [],
				preview_media: [],
				created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
				updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			},
		},
	};
}
function parameters(provider = "hosted:cloudflare", price: unknown = 1000) {
	return {
		provider: {
			provider_name: provider,
			model_id: "@cf/black-forest-labs/flux-1-schnell",
			params: { tier: "FREE" },
		},
		pricing: { request_micro_usd: price },
	};
}

describe("hosted image generation", () => {
	test("switching from local clears weight dependencies, settings and source metadata", () => {
		const preset = defaultGenerationPreset("image");
		const assets = createGenerationAssetDrafts(preset.id, draft);
		const local = applyGenerationModelPreset(draft(), preset.id, assets);
		const hosted = createHostedImageBit(local);
		expect(isHostedImageGeneration(hosted)).toBe(true);
		expect(hosted.dependencies).toEqual([]);
		expect(hosted.parameters.assets).toBeUndefined();
		expect(hosted.parameters.provider.params).toBeUndefined();
		expect(hosted.parameters.pricing).toEqual({});
		expect(hosted.file_name).toBeNull();
		expect(hosted.download_link).toBeNull();
		expect(hosted.size).toBe(0);
		expect(hosted.meta.en.name).toBe("Hosted image generation");
		expect(hosted.meta.en.tags).toEqual(["image-generation", "hosted"]);
		expect(hosted.repository).toBeNull();
		expect(hosted.authors).toEqual([]);
		expect(local.dependencies).toHaveLength(assets.length);
		const restored = applyGenerationModelPreset(hosted, preset.id, assets);
		expect(isHostedImageGeneration(restored)).toBe(false);
		expect(restored.parameters.pricing).toBeUndefined();
		expect(restored.dependencies).toEqual(local.dependencies);
	});

	test("requires an explicit price, model and supported provider without credentials", () => {
		for (const provider of ["hosted:cloudflare", "hosted:openrouter"]) {
			for (const price of [0, 1, Number.MAX_SAFE_INTEGER])
				expect(
					validateHostedImageParameters(parameters(provider, price)),
				).toBeNull();
			for (const price of [
				undefined,
				null,
				-1,
				0.5,
				"0",
				Number.MAX_SAFE_INTEGER + 1,
			])
				expect(
					validateHostedImageParameters({
						...parameters(provider),
						pricing: { request_micro_usd: price },
					}),
				).toContain("price");
		}
		for (const provider of ["Hosted", "hosted:openai", "custom:openrouter"])
			expect(validateHostedImageParameters(parameters(provider))).toContain(
				"Choose",
			);
		const missingModel = parameters();
		missingModel.provider.model_id = " ";
		expect(validateHostedImageParameters(missingModel)).toContain("model ID");
		for (const model of ["@cf/black-forest-labs/flux-2-dev", "arbitrary-model"])
			expect(
				validateHostedImageParameters({
					...parameters(),
					provider: { ...parameters().provider, model_id: model },
				}),
			).toContain("Choose FLUX");
		expect(
			validateHostedImageParameters({
				...parameters(),
				provider: {
					...parameters().provider,
					model_id: "@cf/stabilityai/stable-diffusion-xl-base-1.0",
				},
			}),
		).toBeNull();
		for (const settings of [
			{ api_key: "secret" },
			{ endpoint: "https://example.test" },
			{ stablediffusion: {} },
			{ request_params: {} },
		])
			expect(
				validateHostedImageParameters({
					...parameters(),
					provider: { ...parameters().provider, params: settings },
				}),
			).toContain("server");
		expect(
			validateHostedImageParameters({
				...parameters(),
				provider: { ...parameters().provider, api_surface: "ChatCompletions" },
			}),
		).toContain("API surface");
	});

	test("hosted registration never uploads weights and retains reviewed metadata", async () => {
		const source = updateHostedImageBit(draft(), parameters());
		source.meta.en.name = "Reviewed image model";
		let uploads = 0;
		const root = await prepareGenerationModelRootBit(source, async () => {
			uploads += 1;
			throw new Error("must not upload weights");
		});
		expect(uploads).toBe(0);
		expect(root.meta.en.name).toBe("Reviewed image model");
		expect(root.parameters).toEqual(parameters());
		expect(root.dependencies).toEqual([]);
		const invalid = updateHostedImageBit(source, {
			...parameters(),
			pricing: {},
		});
		await expect(
			prepareGenerationModelRootBit(invalid, async () => {
				uploads += 1;
				return [];
			}),
		).rejects.toThrow("price");
		expect(uploads).toBe(0);
	});

	test("blank tier uses the server default and malformed JSON cannot bypass provider validation", () => {
		const original = parameters();
		const cleared = updateHostedImageTier(original, " ");
		expect(cleared.provider).toEqual({ ...original.provider, params: {} });
		expect(validateHostedImageParameters(cleared)).toBeNull();
		expect(original.provider.params.tier).toBe("FREE");
		for (const tier of ["", " ", null, 4])
			expect(
				validateHostedImageParameters({
					...original,
					provider: { ...original.provider, params: { tier } },
				}),
			).toContain("Access tier");
		for (const params of [[], "FREE", 5])
			expect(
				validateHostedImageParameters({
					...original,
					provider: { ...original.provider, params },
				}),
			).toContain("settings");
		for (const model of [
			"../model",
			"model//id",
			"model?query=1",
			"https://example.test",
			"model with spaces",
			"x".repeat(257),
		])
			expect(
				validateHostedImageParameters({
					...original,
					provider: { provider_name: "hosted:openrouter", model_id: model },
				}),
			).toContain("model ID");
		expect(
			validateHostedImageParameters({
				...original,
				pricing: {
					request_micro_usd: 0,
					input_micro_usd_per_million_tokens: 0,
				},
			}),
		).toContain("per-image price");
	});

	test("local image and video registration still uses the registered weight IDs", async () => {
		for (const kind of ["image", "video"] as const) {
			const preset = defaultGenerationPreset(kind);
			const assets = createGenerationAssetDrafts(preset.id, draft);
			const local = applyGenerationModelPreset(draft(), preset.id, assets);
			const registered = assets.map((asset) => ({
				...asset,
				bit: { ...asset.bit, id: `registered-${asset.bit.id}` },
			}));
			let uploads = 0;
			const root = await prepareGenerationModelRootBit(local, async () => {
				uploads += 1;
				return registered;
			});
			expect(uploads).toBe(1);
			expect(root.dependencies).toEqual(
				registered.map((asset) => asset.bit.id),
			);
			expect(root.type).toBe(local.type);
		}
	});
});
