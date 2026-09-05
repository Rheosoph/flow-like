import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	GENERATION_MODEL_PRESETS,
	applyGenerationModelPreset,
	buildGenerationModelRootBit,
	createGenerationAssetDrafts,
	defaultGenerationPreset,
	getGenerationModelPreset,
	validateGenerationAssets,
} from "./generation-model-preset";

let nextId = 0;
function draft(): IBit {
	return {
		id: `draft-${++nextId}`,
		hub: "",
		hash: "",
		created: "2026-09-05T00:00:00Z",
		updated: "2026-09-05T00:00:00Z",
		authors: [],
		dependencies: [],
		dependency_tree_hash: "",
		parameters: {},
		type: IBitTypes.File,
		meta: {
			en: {
				name: "Draft",
				description: "Draft description",
				preview_media: [],
				tags: [],
				created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
				updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			},
		},
	};
}

describe("generation model presets", () => {
	test("offers image and video defaults and fails on unknown selections", () => {
		expect(defaultGenerationPreset("image").kind).toBe("image");
		expect(defaultGenerationPreset("video").kind).toBe("video");
		expect(() => getGenerationModelPreset("missing-model")).toThrow("Unknown");
		expect(
			new Set(GENERATION_MODEL_PRESETS.map((preset) => preset.id)).size,
		).toBe(GENERATION_MODEL_PRESETS.length);
	});

	for (const preset of GENERATION_MODEL_PRESETS) {
		test(`${preset.label} provides pinned files and supported sampling defaults`, () => {
			const assets = createGenerationAssetDrafts(preset.id, draft);
			expect(validateGenerationAssets(assets)).toEqual([]);
			expect(assets.length).toBeGreaterThan(1);
			for (const asset of assets) {
				expect(asset.bit.download_link).toMatch(
					/^https:\/\/huggingface\.co\/[^/]+\/[^/]+\/resolve\/[a-f0-9]{40}\//,
				);
				expect(asset.bit.size).toBeGreaterThan(0);
				expect(asset.bit.type).toBe(IBitTypes.File);
			}
			expect(preset.defaults.width % 8).toBe(0);
			expect(preset.defaults.height % 8).toBe(0);
			expect(preset.defaults.steps).toBeGreaterThan(0);
			expect(preset.defaults.steps).toBeLessThanOrEqual(100);
			if (preset.kind === "video") {
				expect(((preset.defaults.video_frames ?? 0) - 1) % 4).toBe(0);
				expect(preset.defaults.fps).toBeGreaterThan(0);
				expect(["avi", "webp", "webm"]).toContain(
					preset.defaults.output_format,
				);
			}
		});
	}

	test("publishes a virtual root with returned hub IDs and reviewed metadata", () => {
		const preset = defaultGenerationPreset("image");
		const assets = createGenerationAssetDrafts(preset.id, draft);
		const root = applyGenerationModelPreset(draft(), preset.id, assets);
		root.meta.en.name = "Our reviewed image model";
		root.meta.en.preview_media = ["https://example.com/preview.png"];
		root.parameters.provider.params.generation_defaults.steps = 7;
		const registered = assets.map((asset, index) => ({
			...asset,
			bit: { ...asset.bit, id: `stored-${index}`, hub: "models.example.com" },
		}));
		const published = buildGenerationModelRootBit(root, registered);
		expect(published.type).toBe(IBitTypes.ImageGeneration);
		expect(published.model_slug).toBeNull();
		expect(published.parameters.provider.model_id).toBe(preset.id);
		expect(published.download_link).toBeNull();
		expect(published.file_name).toBeNull();
		expect(published.size).toBe(0);
		expect(published.meta.en.name).toBe("Our reviewed image model");
		expect(published.meta.en.preview_media).toEqual([
			"https://example.com/preview.png",
		]);
		expect(published.parameters.provider.params.generation_defaults.steps).toBe(
			7,
		);
		expect(published.parameters.assets).toEqual(
			registered.map((asset, index) => ({
				role: asset.role,
				bit: `models.example.com:stored-${index}`,
			})),
		);
		expect(published.dependencies).toEqual(
			registered.map((_, index) => `models.example.com:stored-${index}`),
		);
	});

	test("switching presets replaces modality, asset references and provider settings", () => {
		const imagePreset = defaultGenerationPreset("image");
		const videoPreset = defaultGenerationPreset("video");
		const imageAssets = createGenerationAssetDrafts(imagePreset.id, draft);
		const image = applyGenerationModelPreset(
			draft(),
			imagePreset.id,
			imageAssets,
		);
		image.parameters.provider.params.stablediffusion.model_path = "/old/model";
		image.parameters.provider.params.stablediffusion.endpoint =
			"http://old-server";
		const videoAssets = createGenerationAssetDrafts(videoPreset.id, draft);
		const video = applyGenerationModelPreset(
			image,
			videoPreset.id,
			videoAssets,
		);
		expect(video.id).toBe(image.id);
		expect(video.type).toBe(IBitTypes.VideoGeneration);
		expect(
			video.parameters.provider.params.stablediffusion.model_path,
		).toBeUndefined();
		expect(
			video.parameters.provider.params.stablediffusion.endpoint,
		).toBeUndefined();
		expect(video.dependencies).toEqual(
			videoAssets.map((asset) => asset.bit.id),
		);
		expect(video.parameters.provider.params.generation_defaults).toEqual(
			videoPreset.defaults,
		);
	});

	test("rejects incomplete uploads before registering the model root", () => {
		const preset = defaultGenerationPreset("video");
		const assets = createGenerationAssetDrafts(preset.id, draft);
		const root = applyGenerationModelPreset(draft(), preset.id, assets);
		const withoutDecoder = assets.filter((asset) => asset.role !== "vae");
		expect(() => buildGenerationModelRootBit(root, withoutDecoder)).toThrow(
			"Missing required model asset: vae",
		);
		expect(() => buildGenerationModelRootBit(root, [])).toThrow("requires");
	});

	test("rejects ambiguous roles, duplicate file IDs and unsafe upload inputs", () => {
		const preset = defaultGenerationPreset("image");
		const assets = createGenerationAssetDrafts(preset.id, draft);
		const firstAsset = assets[0];
		if (!firstAsset) throw new Error("Preset has no model files");
		expect(
			validateGenerationAssets([...assets, firstAsset]).join(" "),
		).toContain("Duplicate generation asset role");
		const reused = assets.map((asset) => ({
			...asset,
			bit: { ...asset.bit, id: "same" },
		}));
		expect(validateGenerationAssets(reused).join(" ")).toContain(
			"unique Bit ID",
		);
		for (const downloadLink of [
			"",
			"file:///tmp/model",
			"https://user:token@example.com/model",
		]) {
			const invalid = assets.map((asset) => ({
				...asset,
				bit: { ...asset.bit, download_link: downloadLink },
			}));
			expect(validateGenerationAssets(invalid).join(" ")).toContain(
				"HTTPS download URL",
			);
		}
		firstAsset.bit.file_name = "../model.safetensors";
		expect(validateGenerationAssets(assets).join(" ")).toContain(".. segments");
	});
});
