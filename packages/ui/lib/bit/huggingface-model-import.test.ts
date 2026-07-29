import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema";
import {
	applyHuggingFaceMlxImportToUserBit,
	createHuggingFaceGgufAdminDraft,
	createHuggingFaceUserMlxManifest,
	inspectHuggingFaceModelRepository,
	parseHuggingFaceModelReferenceWithPath,
	resolveHuggingFaceGgufSelection,
} from "./huggingface-model-import";

const revision = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const repoId = "owner/model-GGUF";

function jsonResponse(value: unknown, init: ResponseInit = {}): Response {
	return new Response(JSON.stringify(value), {
		status: 200,
		...init,
		headers: {
			"content-type": "application/json",
			...init.headers,
		},
	});
}

function treeFile(path: string, size = 100) {
	return { type: "file", path, size, oid: `oid-${path}` };
}

const ggufTree = [
	treeFile("README.md"),
	treeFile("config.json"),
	treeFile("tokenizer_config.json"),
	treeFile("model-Q8_0.gguf", 800),
	treeFile("model-Q4_K_M.gguf", 400),
	treeFile("model-Q5_K_M-00001-of-00002.gguf", 250),
	treeFile("model-Q5_K_M-00002-of-00002.gguf", 250),
	treeFile("model-IQ3_M-00001-of-00002.gguf", 150),
	treeFile("mmproj-BF16.gguf", 90),
	treeFile("mmproj-F16.gguf", 80),
];

function ggufFetch(
	infoOverrides: Record<string, unknown> = {},
	tree = ggufTree,
) {
	return async (input: string): Promise<Response> => {
		const url = new URL(input);
		if (url.pathname === `/api/models/${repoId}`) {
			return jsonResponse({
				id: repoId,
				author: "owner",
				sha: revision,
				private: false,
				gated: false,
				pipeline_tag: "image-text-to-text",
				tags: ["gguf", "license:apache-2.0"],
				cardData: { license: "apache-2.0" },
				...infoOverrides,
			});
		}
		if (url.pathname.includes("/tree/")) return jsonResponse(tree);
		const file = decodeURIComponent(
			url.pathname.split(`/resolve/${revision}/`)[1] ?? "",
		);
		if (file === "config.json") {
			return jsonResponse({
				architectures: ["LlavaForConditionalGeneration"],
				max_position_embeddings: 32_768,
				vision_config: {},
			});
		}
		if (file === "tokenizer_config.json") {
			return jsonResponse({ model_max_length: 32_768 });
		}
		throw new Error(`Unexpected fixture request: ${input}`);
	};
}

function draft(type = IBitTypes.Llm): IBit {
	return {
		id: "draft",
		type,
		meta: { en: { name: "", description: "", tags: [] } },
		authors: [],
		repository: "",
		download_link: "",
		file_name: "",
		hash: "",
		size: 0,
		hub: "",
		parameters: {
			model_classification: { speed: 0.3 },
			provider: { provider_name: "Local", params: {} },
		},
		version: "0.0.1",
		license: "",
		dependencies: [],
		dependency_tree_hash: "",
		created: "",
		updated: "",
	} as IBit;
}

describe("generic Hugging Face references", () => {
	test("preserves explicitly selected blob and resolve files", () => {
		expect(
			parseHuggingFaceModelReferenceWithPath(
				`https://huggingface.co/${repoId}/resolve/main/weights/model%20Q4.gguf?download=true`,
			),
		).toEqual({
			repoId,
			requestedPath: "weights/model Q4.gguf",
		});
		expect(
			parseHuggingFaceModelReferenceWithPath(
				`https://huggingface.co/${repoId}/blob/main/model-Q8_0.gguf`,
			),
		).toEqual({ repoId, requestedPath: "model-Q8_0.gguf" });
		expect(parseHuggingFaceModelReferenceWithPath(repoId)).toEqual({ repoId });
	});

	test("returns MLX through the same discriminated discovery API", async () => {
		const mlxRepoId = "owner/model-MLX";
		const fetcher = async (input: string): Promise<Response> => {
			const url = new URL(input);
			if (url.pathname === `/api/models/${mlxRepoId}`) {
				return jsonResponse({
					id: mlxRepoId,
					author: "owner",
					sha: revision,
					private: false,
					gated: false,
					pipeline_tag: "text-generation",
					library_name: "mlx",
					tags: ["mlx", "license:apache-2.0"],
				});
			}
			if (url.pathname.includes("/tree/")) {
				return jsonResponse([
					treeFile("config.json"),
					treeFile("model.safetensors", 1_000),
					treeFile("tokenizer.json"),
					treeFile("tokenizer_config.json"),
				]);
			}
			const file = decodeURIComponent(
				url.pathname.split(`/resolve/${revision}/`)[1] ?? "",
			);
			if (file === "config.json") {
				return jsonResponse({
					architectures: ["LlamaForCausalLM"],
					max_position_embeddings: 8192,
				});
			}
			if (file === "tokenizer_config.json") return jsonResponse({});
			throw new Error(`Unexpected fixture request: ${input}`);
		};

		const imported = await inspectHuggingFaceModelRepository(
			mlxRepoId,
			fetcher,
		);
		expect(imported).toMatchObject({
			format: "mlx",
			repoId: mlxRepoId,
			revision,
			kind: "llm",
			contextLength: 8192,
			access: { private: false, gated: false },
		});
		if (imported.format !== "mlx") throw new Error("expected MLX import");
		expect(imported.assets).toHaveLength(4);
	});
});

describe("GGUF repository discovery", () => {
	test("groups quantizations, projectors, and recommends Q4_K_M", async () => {
		const imported = await inspectHuggingFaceModelRepository(
			`https://huggingface.co/${repoId}`,
			ggufFetch(),
		);
		expect(imported.format).toBe("gguf");
		if (imported.format !== "gguf") throw new Error("expected GGUF import");

		expect(imported.kind).toBe("vlm");
		expect(imported.contextLength).toBe(32_768);
		expect(imported.recommendedVariantId).toBe("model-Q4_K_M.gguf");
		expect(imported.recommendedProjectorPath).toBe("mmproj-F16.gguf");
		expect(imported.variants).toEqual(
			expect.arrayContaining([
				expect.objectContaining({
					id: "model-Q4_K_M.gguf",
					quantization: "Q4_K_M",
					split: false,
					complete: true,
				}),
				expect.objectContaining({
					id: "model-Q5_K_M.gguf",
					quantization: "Q5_K_M",
					split: true,
					complete: true,
					files: expect.arrayContaining([
						expect.objectContaining({
							downloadUrl: expect.stringContaining(`/resolve/${revision}/`),
						}),
					]),
				}),
				expect.objectContaining({
					id: "model-IQ3_M.gguf",
					split: true,
					complete: false,
				}),
			]),
		);
		expect(imported.ignoredPaths).toEqual([
			"config.json",
			"README.md",
			"tokenizer_config.json",
		]);
	});

	test("an explicit file wins over the automatic quantization recommendation", async () => {
		const imported = await inspectHuggingFaceModelRepository(
			`https://huggingface.co/${repoId}/resolve/main/model-Q8_0.gguf`,
			ggufFetch(),
		);
		if (imported.format !== "gguf") throw new Error("expected GGUF import");
		expect(imported.requestedPath).toBe("model-Q8_0.gguf");
		expect(imported.recommendedVariantId).toBe("model-Q8_0.gguf");
	});

	test("rejects incomplete and split variants with actionable errors", async () => {
		const imported = await inspectHuggingFaceModelRepository(
			repoId,
			ggufFetch(),
		);
		if (imported.format !== "gguf") throw new Error("expected GGUF import");
		expect(() =>
			resolveHuggingFaceGgufSelection(imported, {
				variantId: "model-IQ3_M.gguf",
			}),
		).toThrow("incomplete split");
		expect(() =>
			resolveHuggingFaceGgufSelection(imported, {
				variantId: "model-Q5_K_M.gguf",
			}),
		).toThrow("split across");
	});

	test("requires an explicit model kind when metadata is inconclusive", async () => {
		const imported = await inspectHuggingFaceModelRepository(
			repoId,
			ggufFetch(
				{ pipeline_tag: null },
				ggufTree.filter(
					(file) =>
						file.path !== "config.json" &&
						file.path !== "tokenizer_config.json" &&
						!file.path.startsWith("mmproj"),
				),
			),
		);
		if (imported.format !== "gguf") throw new Error("expected GGUF import");
		expect(imported.kind).toBe("unknown");
		expect(() => resolveHuggingFaceGgufSelection(imported)).toThrow(
			"Choose whether",
		);
		expect(
			resolveHuggingFaceGgufSelection(imported, { kind: "llm" }).kind,
		).toBe("llm");
	});

	test("rejects private and gated repositories until authentication exists", async () => {
		await expect(
			inspectHuggingFaceModelRepository(repoId, ggufFetch({ gated: "manual" })),
		).rejects.toThrow("authentication flow");
		await expect(
			inspectHuggingFaceModelRepository(repoId, ggufFetch({ private: true })),
		).rejects.toThrow("authentication flow");
	});
});

describe("Hugging Face Bit mappers", () => {
	test("creates an admin GGUF root and projector with immutable source URLs", async () => {
		const imported = await inspectHuggingFaceModelRepository(
			repoId,
			ggufFetch(),
		);
		if (imported.format !== "gguf") throw new Error("expected GGUF import");
		const mapped = createHuggingFaceGgufAdminDraft(draft(), imported, () => ({
			...draft(IBitTypes.Projection),
			id: "projector",
		}));

		expect(mapped.root).toMatchObject({
			type: IBitTypes.Vlm,
			file_name: "model-Q4_K_M.gguf",
			size: 400,
			repository: `https://huggingface.co/${repoId}`,
			parameters: {
				context_length: 32_768,
				provider: {
					provider_name: "Local",
					model_id: repoId,
					version: revision,
				},
			},
		});
		expect(mapped.root.download_link).toContain(`/resolve/${revision}/`);
		expect(mapped.projection).toMatchObject({
			id: "projector",
			type: IBitTypes.Projection,
			file_name: "mmproj-F16.gguf",
			size: 80,
		});
	});

	test("stores a canonical top-level user MLX manifest without source URLs", () => {
		const imported = {
			repoId: "owner/mlx-model",
			repositoryUrl: "https://huggingface.co/owner/mlx-model",
			revision,
			kind: "llm" as const,
			kindEvidence: ["Hub task: text-generation"],
			modelName: "mlx-model",
			author: "owner",
			authorUrl: "https://huggingface.co/owner",
			license: "apache-2.0",
			tags: ["mlx"],
			contextLength: 8192,
			assets: [
				{
					path: "config.json",
					size: 100,
					downloadUrl: `https://huggingface.co/owner/mlx-model/resolve/${revision}/config.json?download=true`,
					oid: "git-oid",
					lfsOid: "lfs-oid",
				},
			],
			ignoredPaths: [],
			totalSize: 100,
			warnings: [],
		};
		const manifest = createHuggingFaceUserMlxManifest(imported);
		expect(manifest).toEqual({
			schema: 1,
			repo_id: "owner/mlx-model",
			revision,
			format: "mlx",
			files: [
				{
					path: "config.json",
					size: 100,
					oid: "git-oid",
					lfs_oid: "lfs-oid",
				},
			],
		});
		expect(manifest.files[0]).not.toHaveProperty("download_link");

		const bit = applyHuggingFaceMlxImportToUserBit(draft(), imported);
		expect(bit).toMatchObject({
			type: IBitTypes.Llm,
			download_link: null,
			file_name: null,
			size: 0,
			dependencies: [],
			parameters: {
				huggingface: manifest,
				provider: {
					provider_name: "MLX",
					model_id: "owner/mlx-model",
					version: revision,
					params: {},
				},
			},
		});
		expect(bit.parameters.provider.params).not.toHaveProperty("huggingface");
	});
});
