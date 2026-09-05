import { describe, expect, test } from "bun:test";
import type { GenerationAssetDraft, IBit } from "@flow-like/flow-like-ui";
import { registerGenerationAssets as registerWebAssets } from "../../../../../web/app/admin/bits/add/generation-upload";
import { registerGenerationAssets as registerDesktopAssets } from "./generation-upload";

function draft(
	key: string,
	role: GenerationAssetDraft["role"],
): GenerationAssetDraft {
	return {
		key,
		role,
		bit: {
			id: key,
			hub: "",
			hash: "",
			authors: [],
			created: "2026-09-05T00:00:00Z",
			updated: "2026-09-05T00:00:00Z",
			dependencies: [],
			dependency_tree_hash: "",
			parameters: {},
			type: "File" as IBit["type"],
			meta: {},
			file_name: `${role}.safetensors`,
			download_link: `https://models.example/${role}.safetensors`,
		},
	};
}

for (const [app, registerGenerationAssets] of [
	["desktop", registerDesktopAssets],
	["web", registerWebAssets],
] as const) {
	describe(`${app} generation model registration`, () => {
		test("resumes an interrupted upload with returned IDs and preserves asset order", async () => {
			let assets = [
				draft("weights", "diffusion_model"),
				draft("decoder", "vae"),
			];
			const completed = new Map<string, string>();
			const progress: number[] = [];
			const uploads: string[] = [];
			let failDecoder = true;
			const callbacks = {
				completed,
				uploadBit: async (bit: IBit) => {
					uploads.push(bit.id);
					if (bit.id === "decoder" && failDecoder)
						throw new Error("Connection lost");
					return { ...bit, id: `registered-${bit.id}`, hub: "models.example" };
				},
				onProgress: (_asset: GenerationAssetDraft, index: number) => {
					progress.push(index);
				},
				onRegistered: (asset: GenerationAssetDraft) => {
					assets = assets.map((current) =>
						current.key === asset.key ? asset : current,
					);
				},
			};

			await expect(
				registerGenerationAssets({ ...callbacks, assets }),
			).rejects.toThrow("Connection lost");
			expect(assets[0].bit.id).toBe("registered-weights");
			expect(assets[1].bit.id).toBe("decoder");
			failDecoder = false;
			const registered = await registerGenerationAssets({
				...callbacks,
				assets,
			});
			expect(uploads).toEqual(["weights", "decoder", "decoder"]);
			expect(progress).toEqual([0, 1, 1]);
			expect(
				registered.map((asset) => `${asset.bit.hub}:${asset.bit.id}`),
			).toEqual([
				"models.example:registered-weights",
				"models.example:registered-decoder",
			]);
			expect(registered.map((asset) => asset.role)).toEqual([
				"diffusion_model",
				"vae",
			]);
			await registerGenerationAssets({ ...callbacks, assets });
			expect(uploads).toHaveLength(3);
		});

		test("registers an edited URL again while retaining completed files", async () => {
			let assets = [
				draft("weights", "diffusion_model"),
				draft("decoder", "vae"),
			];
			const uploads: string[] = [];
			const callbacks = {
				completed: new Map<string, string>(),
				uploadBit: async (bit: IBit) => {
					uploads.push(bit.download_link ?? "");
					return { ...bit, hub: "models.example" };
				},
				onProgress: () => {},
				onRegistered: (asset: GenerationAssetDraft) => {
					assets = assets.map((current) =>
						current.key === asset.key ? asset : current,
					);
				},
			};
			await registerGenerationAssets({ ...callbacks, assets });
			assets = assets.map((asset) =>
				asset.key === "weights"
					? {
							...asset,
							bit: {
								...asset.bit,
								download_link: "https://models.example/revised.safetensors",
							},
						}
					: asset,
			);
			await registerGenerationAssets({ ...callbacks, assets });
			expect(uploads).toEqual([
				"https://models.example/diffusion_model.safetensors",
				"https://models.example/vae.safetensors",
				"https://models.example/revised.safetensors",
			]);
		});
	});
}
