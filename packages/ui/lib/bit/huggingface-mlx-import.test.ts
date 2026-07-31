import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema";
import {
	applyHuggingFaceMlxImportToBit,
	createHuggingFaceMlxAssetBits,
	huggingFacePinnedDownloadUrl,
	inspectHuggingFaceMlxRepository,
	parseHuggingFaceModelReference,
	validateHuggingFacePinnedGgufDownloadUrl,
} from "./huggingface-mlx-import";

const revision = "d928611c119d7d86037d6926ac50712062ff0f29";
const repoId = "unsloth/Qwen3.6-35B-A3B-MLX-8bit";

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

function modelInfo(overrides: Record<string, unknown> = {}) {
	return {
		id: repoId,
		author: "unsloth",
		sha: revision,
		private: false,
		gated: false,
		pipeline_tag: "image-text-to-text",
		library_name: "mlx",
		tags: ["mlx", "safetensors", "license:apache-2.0", "region:us"],
		cardData: { license: "apache-2.0" },
		...overrides,
	};
}

function treeFile(path: string, size = 12) {
	return { type: "file", path, size, oid: `oid-${path}` };
}

const minimalVlmTree = [
	treeFile(".gitattributes"),
	treeFile("README.md"),
	treeFile("chat_template.jinja"),
	treeFile("config.json"),
	treeFile("model.safetensors", 1024),
	treeFile("processor_config.json"),
	treeFile("tokenizer.json"),
	treeFile("tokenizer_config.json"),
];

function fixtureFetch(
	info: Record<string, unknown>,
	tree = minimalVlmTree,
	jsonFiles: Record<string, unknown> = {
		"config.json": {
			architectures: ["QwenForConditionalGeneration"],
			text_config: { max_position_embeddings: 262_144 },
			vision_config: { model_type: "qwen_vision" },
		},
		"tokenizer_config.json": { model_max_length: 262_144 },
		"processor_config.json": { processor_class: "QwenVLProcessor" },
	},
) {
	return async (input: string): Promise<Response> => {
		const url = new URL(input);
		if (url.pathname === `/api/models/${repoId}`) return jsonResponse(info);
		if (url.pathname.includes("/tree/")) return jsonResponse(tree);
		const file = url.pathname.split(`/resolve/${revision}/`)[1];
		if (file && jsonFiles[decodeURIComponent(file)]) {
			return jsonResponse(jsonFiles[decodeURIComponent(file)]);
		}
		throw new Error(`Unexpected fixture request: ${input}`);
	};
}

describe("Hugging Face model references", () => {
	test("accepts repository ids and canonical Hub URLs", () => {
		expect(parseHuggingFaceModelReference(repoId)).toBe(repoId);
		expect(
			parseHuggingFaceModelReference(
				`https://huggingface.co/${repoId}/tree/main?foo=bar`,
			),
		).toBe(repoId);
		expect(
			parseHuggingFaceModelReference(
				`https://huggingface.co/${repoId}/blob/main/config.json`,
			),
		).toBe(repoId);
	});

	test("rejects lookalike origins, credentials, and invalid ids", () => {
		for (const reference of [
			"https://huggingface.co.example/owner/model",
			"https://user:token@huggingface.co/owner/model",
			"http://huggingface.co/owner/model",
			"owner",
			"owner/model/extra",
			"../model",
		]) {
			expect(() => parseHuggingFaceModelReference(reference)).toThrow();
		}
	});

	test("builds immutable, path-encoded resolver URLs", () => {
		const url = huggingFacePinnedDownloadUrl(
			repoId,
			revision,
			"weights shard/model 01.safetensors",
		);
		expect(url).toBe(
			`https://huggingface.co/${repoId}/resolve/${revision}/weights%20shard/model%2001.safetensors?download=true`,
		);
		expect(() =>
			huggingFacePinnedDownloadUrl(repoId, "main", "config.json"),
		).toThrow("immutable revision");
	});

	test("accepts only credential-free, SHA-pinned Hugging Face GGUF URLs", () => {
		expect(() =>
			validateHuggingFacePinnedGgufDownloadUrl(
				huggingFacePinnedDownloadUrl(
					repoId,
					revision,
					"weights/model Q4_K_M.gguf",
				),
			),
		).not.toThrow();

		for (const reference of [
			`http://huggingface.co/${repoId}/resolve/${revision}/model.gguf`,
			`https://huggingface.co.example/${repoId}/resolve/${revision}/model.gguf`,
			`https://user:token@huggingface.co/${repoId}/resolve/${revision}/model.gguf`,
			`https://huggingface.co/${repoId}/resolve/main/model.gguf`,
			`https://huggingface.co/${repoId}/blob/${revision}/model.gguf`,
			`https://huggingface.co/${repoId}/resolve/${revision}/model.gguf?token=secret`,
			`https://huggingface.co/${repoId}/resolve/${revision}/model.gguf#mutable`,
			`https://huggingface.co/${repoId}/resolve/${revision}/config.json`,
			`https://huggingface.co/${repoId}/resolve/${revision}/%2e%2e%2fmodel.gguf`,
		]) {
			expect(() =>
				validateHuggingFacePinnedGgufDownloadUrl(reference),
			).toThrow();
		}
	});
});

describe("Hugging Face MLX repository inspection", () => {
	test("turns the supplied repository shape into a pinned VLM manifest", async () => {
		const imported = await inspectHuggingFaceMlxRepository(
			`https://huggingface.co/${repoId}`,
			fixtureFetch(modelInfo()),
		);

		expect(imported).toMatchObject({
			repoId,
			revision,
			kind: "vlm",
			modelName: "Qwen3.6-35B-A3B-MLX-8bit",
			author: "unsloth",
			license: "apache-2.0",
			contextLength: 262_144,
			architecture: "QwenForConditionalGeneration",
		});
		expect(imported.assets.map((asset) => asset.path)).toEqual([
			"chat_template.jinja",
			"config.json",
			"model.safetensors",
			"processor_config.json",
			"tokenizer.json",
			"tokenizer_config.json",
		]);
		expect(imported.ignoredPaths).toEqual([".gitattributes", "README.md"]);
		expect(
			imported.assets.every((asset) =>
				asset.downloadUrl.includes(`/resolve/${revision}/`),
			),
		).toBe(true);

		const draft = {
			id: "root",
			type: IBitTypes.Llm,
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
				provider: { provider_name: "Local" },
			},
			version: "0.0.1",
			license: "",
			dependencies: [],
			dependency_tree_hash: "",
			created: "",
			updated: "",
		} as IBit;
		const root = applyHuggingFaceMlxImportToBit(draft, imported);
		expect(root).toMatchObject({
			type: IBitTypes.Vlm,
			name: "Qwen3.6-35B-A3B-MLX-8bit",
			repository: `https://huggingface.co/${repoId}`,
			download_link: null,
			file_name: null,
			size: 0,
			license: "apache-2.0",
			parameters: {
				context_length: 262_144,
				model_classification: { speed: 0.3 },
				provider: {
					provider_name: "MLX",
					model_id: repoId,
					version: revision,
				},
			},
		});
		const bits = createHuggingFaceMlxAssetBits(imported, (fileName) => ({
			...draft,
			id: `asset-${fileName}`,
			file_name: fileName,
		}));
		expect(bits).toHaveLength(imported.assets.length);
		expect(
			bits.find((entry) => entry.file_name === "config.json"),
		).toMatchObject({
			type: IBitTypes.Config,
			size: 12,
		});
	});

	test("classifies text-generation repositories without vision signals as LLMs", async () => {
		const imported = await inspectHuggingFaceMlxRepository(
			repoId,
			fixtureFetch(
				modelInfo({ pipeline_tag: "text-generation" }),
				minimalVlmTree,
				{
					"config.json": {
						architectures: ["LlamaForCausalLM"],
						max_position_embeddings: 32_768,
					},
					"tokenizer_config.json": {},
					"processor_config.json": {
						processor_class: "ProcessorMixin",
					},
				},
			),
		);
		expect(imported.kind).toBe("llm");
		expect(imported.contextLength).toBe(32_768);
	});

	test("uses a safetensors index and rejects missing or unsafe shards", async () => {
		const indexedTree = [
			...minimalVlmTree.filter((file) => file.path !== "model.safetensors"),
			treeFile("model.safetensors.index.json"),
			treeFile("model-00001-of-00002.safetensors", 100),
			treeFile("model-00002-of-00002.safetensors", 100),
			treeFile("unused.safetensors", 100),
		];
		const imported = await inspectHuggingFaceMlxRepository(
			repoId,
			fixtureFetch(modelInfo(), indexedTree, {
				"config.json": { vision_config: {} },
				"tokenizer_config.json": {},
				"processor_config.json": { image_processor: {} },
				"model.safetensors.index.json": {
					weight_map: {
						a: "model-00001-of-00002.safetensors",
						b: "model-00002-of-00002.safetensors",
					},
				},
			}),
		);
		expect(
			imported.assets.some((asset) => asset.path === "unused.safetensors"),
		).toBe(false);

		await expect(
			inspectHuggingFaceMlxRepository(
				repoId,
				fixtureFetch(modelInfo(), indexedTree, {
					"config.json": { vision_config: {} },
					"tokenizer_config.json": {},
					"processor_config.json": { image_processor: {} },
					"model.safetensors.index.json": {
						weight_map: { a: "../outside.safetensors" },
					},
				}),
			),
		).rejects.toThrow("unsafe shard path");
	});

	test("follows safe tree pagination", async () => {
		let treeCalls = 0;
		const fetcher = async (input: string): Promise<Response> => {
			const url = new URL(input);
			if (url.pathname === `/api/models/${repoId}`) {
				return jsonResponse(modelInfo({ pipeline_tag: "text-generation" }));
			}
			if (url.pathname.includes("/tree/")) {
				treeCalls += 1;
				if (treeCalls === 1) {
					return jsonResponse(
						[treeFile("config.json"), treeFile("model.safetensors", 100)],
						{
							headers: {
								link: `<https://huggingface.co/api/models/${repoId}/tree/${revision}?recursive=true&cursor=next>; rel="next"`,
							},
						},
					);
				}
				return jsonResponse([
					treeFile("tokenizer.json"),
					treeFile("tokenizer_config.json"),
				]);
			}
			const file = decodeURIComponent(
				url.pathname.split(`/resolve/${revision}/`)[1] ?? "",
			);
			if (file === "config.json") {
				return jsonResponse({ max_position_embeddings: 8192 });
			}
			if (file === "tokenizer_config.json") return jsonResponse({});
			throw new Error(`Unexpected fixture request: ${input}`);
		};

		const imported = await inspectHuggingFaceMlxRepository(repoId, fetcher);
		expect(treeCalls).toBe(2);
		expect(imported.kind).toBe("llm");
		expect(imported.assets).toHaveLength(4);
	});

	test("rejects private, gated, non-MLX, and failed API responses", async () => {
		await expect(
			inspectHuggingFaceMlxRepository(
				repoId,
				fixtureFetch(modelInfo({ gated: "manual" })),
			),
		).rejects.toThrow("redistribution");
		await expect(
			inspectHuggingFaceMlxRepository(
				repoId,
				fixtureFetch(
					modelInfo({
						library_name: "transformers",
						tags: ["safetensors"],
						id: "owner/plain-model",
					}),
				),
			),
		).rejects.toThrow("not marked as an MLX model");
		await expect(
			inspectHuggingFaceMlxRepository(
				repoId,
				async () => new Response("rate limited", { status: 429 }),
			),
		).rejects.toThrow("rate-limited");
	});
});
