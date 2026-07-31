import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema";
import {
	buildMlxModelRootBit,
	inferMlxAssetBitType,
	mlxAssetPathError,
	prepareMlxAssetBit,
	validateMlxModelAssets,
} from "./mlx-model-pack";

const asset = (
	fileName: string,
	downloadLink = `https://models/${fileName}`,
) => ({
	file_name: fileName,
	download_link: downloadLink,
});

describe("MLX asset paths", () => {
	test("accepts safe directory-relative paths", () => {
		expect(mlxAssetPathError("weights/model-00001-of-00002.safetensors")).toBe(
			undefined,
		);
	});

	test("rejects absolute, traversal, backslash, and non-portable paths", () => {
		for (const path of [
			"/config.json",
			"../config.json",
			"weights/../config.json",
			"weights\\model.safetensors",
			"C:/config.json",
			"weights//model.safetensors",
			"weights/model\0.safetensors",
		]) {
			expect(mlxAssetPathError(path)).toBeDefined();
		}
	});
});

describe("MLX asset types", () => {
	test("uses specialized bit types for known metadata", () => {
		expect(inferMlxAssetBitType("config.json")).toBe(IBitTypes.Config);
		expect(inferMlxAssetBitType("tokenizer.json")).toBe(IBitTypes.Tokenizer);
		expect(inferMlxAssetBitType("tokenizer_config.json")).toBe(
			IBitTypes.TokenizerConfig,
		);
		expect(inferMlxAssetBitType("preprocessor_config.json")).toBe(
			IBitTypes.PreprocessorConfig,
		);
		expect(inferMlxAssetBitType("weights/model.safetensors")).toBe(
			IBitTypes.File,
		);
	});
});

describe("MLX model manifests", () => {
	const llmAssets = [
		asset("config.json"),
		asset("tokenizer.json"),
		asset("tokenizer_config.json"),
		asset("model.safetensors"),
	];

	test("accepts a minimal LLM bundle", () => {
		expect(validateMlxModelAssets(llmAssets, false)).toEqual([]);
	});

	test("requires VLM processor metadata", () => {
		expect(validateMlxModelAssets(llmAssets, true)).toContain(
			"MLX VLM bundles require processor_config.json or preprocessor_config.json",
		);
		expect(
			validateMlxModelAssets(
				[...llmAssets, asset("preprocessor_config.json")],
				true,
			),
		).toEqual([]);
	});

	test("requires exact root metadata names expected by MLX loaders", () => {
		expect(
			validateMlxModelAssets(
				[
					asset("CONFIG.JSON"),
					asset("tokenizer.json"),
					asset("tokenizer_config.json"),
					asset("model.safetensors"),
				],
				false,
			),
		).toContain("The bundle requires config.json at its root");
		expect(
			validateMlxModelAssets(
				[
					asset("config.json"),
					asset("TOKENIZER.JSON"),
					asset("tokenizer_config.json"),
					asset("model.safetensors"),
				],
				false,
			),
		).toContain("The bundle requires tokenizer.json at its root");
	});

	test("rejects duplicate portable targets and missing URLs", () => {
		const errors = validateMlxModelAssets(
			[...llmAssets, asset("CONFIG.JSON"), asset("generation_config.json", "")],
			false,
		);
		expect(
			errors.some((error) => error.includes("duplicate stored path")),
		).toBe(true);
		expect(
			errors.some((error) => error.includes("download URL is required")),
		).toBe(true);
	});

	test("rejects invalid URLs and file-parent conflicts before upload", () => {
		const errors = validateMlxModelAssets(
			[
				...llmAssets,
				asset("weights", "file:///tmp/weights"),
				asset("weights-model.safetensors"),
				asset("weights/model-00001-of-00002.safetensors"),
			],
			false,
		);
		expect(
			errors.some((error) => error.includes("must use http:// or https://")),
		).toBe(true);
		expect(
			errors.some((error) => error.includes("file parent of the other")),
		).toBe(true);
	});

	test("rejects tokenizer layouts unsupported by the Swift loader", () => {
		const errors = validateMlxModelAssets(
			[
				asset("config.json"),
				asset("vocab.json"),
				asset("merges.txt"),
				asset("weights/model.safetensors"),
			],
			false,
		);
		expect(errors).toContain("The bundle requires tokenizer.json at its root");
		expect(errors).toContain(
			"The bundle requires tokenizer_config.json at its root",
		);
	});
});

describe("MLX Bit pack builder", () => {
	const root = {
		id: "draft-root",
		hub: "",
		type: IBitTypes.Llm,
		created: "2026-01-01T00:00:00.000Z",
		updated: "2026-01-01T00:00:00.000Z",
		hash: "",
		dependency_tree_hash: "",
		meta: {},
		download_link: "https://models/obsolete.safetensors",
		file_name: "obsolete.safetensors",
		size: 123,
		license: "mit",
		authors: ["https://authors/alice"],
		repository: "https://models/repo",
		parameters: {
			provider: {
				provider_name: "MLX",
				model_id: "mlx-community/example",
			},
		},
		dependencies: [],
	} satisfies IBit;

	test("normalizes concrete dependency Bits and inherits root attribution", () => {
		const prepared = prepareMlxAssetBit(
			{
				...root,
				id: "asset",
				file_name: " tokenizer.json ",
				download_link: " https://models/tokenizer.json ",
			},
			root,
		);

		expect(prepared).toMatchObject({
			id: "asset",
			type: IBitTypes.Tokenizer,
			file_name: "tokenizer.json",
			download_link: "https://models/tokenizer.json",
			license: "mit",
			authors: ["https://authors/alice"],
			repository: "https://models/repo",
			parameters: {},
		});
	});

	test("creates an artifact-free virtual root with registered dependencies", () => {
		const built = buildMlxModelRootBit(root, [
			{ hub: "hub.example", id: "config-bit" },
			{ hub: "hub.example", id: "weights-bit" },
		]);

		expect(built).toMatchObject({
			id: "draft-root",
			type: IBitTypes.Llm,
			download_link: null,
			file_name: null,
			size: 0,
			dependencies: ["hub.example:config-bit", "hub.example:weights-bit"],
			parameters: root.parameters,
		});
	});
});
