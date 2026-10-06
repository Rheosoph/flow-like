import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes, IPooling } from "../../../../../lib/schema";
import {
	type ModelCandidate,
	type ModelChoice,
	type ModelOption,
	type OptionFile,
	choiceBlock,
	choiceBytes,
	choiceFacts,
	displayNameOf,
	engineOfBit,
	filesToFingerprint,
	fromHubPack,
	fromUserBit,
	isHostableBit,
	modelIdFor,
	modelSpecOf,
	quantizationOf,
	userBitLinks,
} from "./model-options";

const GIB = 1024 ** 3;

const file = (name: string, size: number, digest = true): OptionFile => ({
	file_name: name,
	size,
	...(digest ? { digest: { algorithm: "sha256", hex: "e".repeat(64) } } : {}),
	sources: [`https://huggingface.co/o/m/resolve/${"1".repeat(40)}/${name}`],
	weights: name.endsWith(".gguf") || name.endsWith(".safetensors"),
});

function option(overrides: Partial<ModelOption> = {}): ModelOption {
	return {
		id: "q4",
		label: "Q4_K_M",
		engine: "llamacpp",
		files: [file("model-Q4_K_M.gguf", 5 * GIB)],
		projectors: [
			file("mmproj-F16.gguf", 0.8 * GIB),
			file("mmproj-BF16.gguf", 0.8 * GIB),
		],
		recommended: true,
		facts: { contextLength: 32_768 },
		...overrides,
	};
}

const candidate = (options = [option()]): ModelCandidate => ({
	source: "huggingface",
	key: "huggingface:o/m@1",
	name: "Gemma-3-4B-it",
	kinds: ["vision", "chat"],
	options,
});

const choice = (overrides: Partial<ModelChoice> = {}): ModelChoice => ({
	candidate: candidate(),
	option: option(),
	kind: "chat",
	...overrides,
});

function bit(overrides: Partial<IBit>): IBit {
	return {
		id: "bit",
		type: IBitTypes.Llm,
		authors: [],
		created: "",
		updated: "",
		dependencies: [],
		dependency_tree_hash: "",
		hash: "",
		hub: "",
		meta: {},
		parameters: { provider: { provider_name: "Local" } },
		...overrides,
	} as IBit;
}

describe("spec", () => {
	test("a chat model: its weights in load order with digests and sources, no projector", () => {
		const spec = modelSpecOf(choice());
		expect(spec).toEqual({
			display_name: "Gemma-3-4B-it Q4_K_M",
			kind: "chat",
			engine: "llamacpp",
			assets: [
				{
					digest: { algorithm: "sha256", hex: "e".repeat(64) },
					size: 5 * GIB,
					file_name: "model-Q4_K_M.gguf",
					sources: [
						`https://huggingface.co/o/m/resolve/${"1".repeat(40)}/model-Q4_K_M.gguf`,
					],
				},
			],
		});
	});

	test("a vision model on llama.cpp loads the chosen projector last", () => {
		const vision = choice({ kind: "vision", projector: "mmproj-BF16.gguf" });
		const spec = modelSpecOf(vision);
		expect(spec.projector).toBe("mmproj-BF16.gguf");
		expect(spec.assets.map((asset) => asset.file_name)).toEqual([
			"model-Q4_K_M.gguf",
			"mmproj-BF16.gguf",
		]);
		expect(modelSpecOf(choice({ kind: "vision" })).projector).toBe(
			"mmproj-F16.gguf",
		);
		expect(choiceBytes(vision)).toBe(5.8 * GIB);
		expect(choiceFacts(vision).weightBytes).toBe(5.8 * GIB);
	});

	test("digests this computer computed fill the files that had none; a file without any fails", () => {
		const mlx = choice({
			option: option({
				engine: "mlx",
				files: [
					file("config.json", 900, false),
					file("model.safetensors", 2 * GIB),
				],
				projectors: [],
			}),
		});
		expect(filesToFingerprint(mlx).map((entry) => entry.file_name)).toEqual([
			"config.json",
		]);
		expect(() => modelSpecOf(mlx)).toThrow(
			"Building the model spec of Gemma-3-4B-it Q4_K_M failed: config.json has no fingerprint.",
		);
		const digest = { algorithm: "sha256", hex: "f".repeat(64) } as const;
		expect(
			modelSpecOf(mlx, new Map([["config.json", digest]])).assets[0]?.digest,
		).toEqual(digest);
		expect(choiceFacts(mlx).weightBytes).toBe(2 * GIB);
	});

	test("an embedding model carries its pooling", () => {
		const embedding = fromHubPack(
			bit({
				id: "bge",
				type: IBitTypes.Embedding,
				hash: "7".repeat(64),
				file_name: "model.onnx",
				download_link: "https://cdn.flow-like.com/bits/bge",
				size: 133_000_000,
				meta: { en: { name: "BGE Small EN v1.5" } as IBit["meta"][string] },
				parameters: {
					input_length: 512,
					pooling: IPooling.Cls,
					provider: { provider_name: "Local" },
				},
			}),
			[],
		);
		const [onnx] = embedding.options;
		if (!onnx) throw new Error("The hub model has no version.");
		const spec = modelSpecOf({
			candidate: embedding,
			option: onnx,
			kind: "embedding",
		});
		expect(spec).toMatchObject({
			display_name: "BGE Small EN v1.5",
			kind: "embedding",
			engine: "onnx",
			pooling: "cls",
			assets: [
				{
					digest: { algorithm: "blake3", hex: "7".repeat(64) },
					file_name: "model.onnx",
					sources: ["https://cdn.flow-like.com/bits/bge"],
				},
			],
		});
		expect(onnx?.facts).toEqual({ contextLength: 512 });
	});
});

describe("sources", () => {
	const revision = "ab".repeat(20);
	const pinned = `https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/${revision}/Qwen3-8B-Q4_K_M.gguf?download=true`;
	const hubQwen = (overrides: Partial<IBit>) =>
		bit({
			id: "qwen",
			hash: "c".repeat(64),
			file_name: "Qwen3-8B-Q4_K_M.gguf",
			size: 5 * GIB,
			parameters: {
				provider: {
					provider_name: "Local",
					model_id: "Qwen/Qwen3-8B-GGUF",
					version: revision.toUpperCase(),
				},
			},
			...overrides,
		});
	const filesOf = (candidate: ModelCandidate) =>
		candidate.options[0]?.files ?? [];

	test("a hub that signs its links: the signed link stays here, the pinned Hugging Face file is the source", () => {
		const [signed] = filesOf(
			fromHubPack(
				hubQwen({
					download_link:
						"https://cdn.flow-like.com/bits/qwen?X-Amz-Expires=86400&X-Amz-Signature=abc",
				}),
				[],
			),
		);
		expect(signed?.sources).toEqual([pinned]);
		expect(signed?.digest).toEqual({
			algorithm: "blake3",
			hex: "c".repeat(64),
		});
		const [open] = filesOf(
			fromHubPack(
				hubQwen({ download_link: "https://cdn.flow-like.com/bits/qwen" }),
				[],
			),
		);
		expect(open?.sources).toEqual([
			"https://cdn.flow-like.com/bits/qwen",
			pinned,
		]);
	});

	test("a hub Bit whose hash is only its id carries no digest", () => {
		const [file] = filesOf(fromHubPack(hubQwen({ hash: "qwen" }), []));
		expect(file?.digest).toBeUndefined();
	});

	test("a user Bit's link with a signature, credentials or a fragment is no source", () => {
		for (const download_link of [
			`${pinned}&X-Amz-Signature=abc`,
			pinned.replace("https://", "https://user:secret@"),
			`${pinned}#weights`,
			"not a link",
		]) {
			const mine = bit({ id: "mine", file_name: "m.gguf", download_link });
			expect(userBitLinks(mine)).toEqual([]);
			expect(filesOf(fromUserBit(mine, new Map()))).toEqual([]);
		}
		const mine = bit({
			id: "mine",
			file_name: "m.gguf",
			download_link: pinned,
		});
		expect(userBitLinks(mine)).toEqual([pinned]);
		expect(filesOf(fromUserBit(mine, new Map()))[0]?.sources).toEqual([pinned]);
	});
});

describe("blocks", () => {
	test("installable: nothing blocks", () => {
		expect(choiceBlock(choice())).toBeUndefined();
	});

	test("a sharded MLX model keeps every descriptor beyond the reply projection", () => {
		const files = Array.from({ length: 33 }, (_, index) => ({
			...file(`model-${index}.safetensors`, GIB),
			digest: {
				algorithm: "sha256" as const,
				hex: index.toString(16).padStart(64, "0"),
			},
		}));
		const sharded = choice({ option: option({ engine: "mlx", files }) });
		expect(choiceBlock(sharded)).toBeUndefined();
		expect(modelSpecOf(sharded).assets).toHaveLength(33);
		expect(modelSpecOf(sharded).assets.at(-1)?.digest).toEqual(
			files.at(-1)?.digest,
		);
	});

	test.each([
		[
			"a vision model on llama.cpp without a projector",
			choice({ kind: "vision", option: option({ projectors: [] }) }),
			"projector_missing",
		],
		[
			"an incomplete split",
			choice({ option: option({ blocked: "split_incomplete" }) }),
			"split_incomplete",
		],
		[
			"a big file without a digest",
			choice({
				option: option({ files: [file("model.gguf", 5 * GIB, false)] }),
			}),
			"no_fingerprint",
		],
		[
			"more files than the device takes",
			choice({
				option: option({
					files: Array.from({ length: 257 }, (_, index) =>
						file(`model-${index}.gguf`, GIB),
					),
				}),
			}),
			"too_many_files",
		],
		[
			"a file over 64 GiB",
			choice({ option: option({ files: [file("model.gguf", 65 * GIB)] }) }),
			"file_too_large",
		],
	] as const)("%s", (_, blocked, reason) => {
		expect(choiceBlock(blocked)).toBe(reason);
	});

	test("a small file without a digest is fingerprinted, not blocked", () => {
		expect(
			choiceBlock(
				choice({
					option: option({
						files: [
							file("model.gguf", GIB),
							file("tokenizer.json", 9_000_000, false),
						],
					}),
				}),
			),
		).toBeUndefined();
	});
});

describe("names", () => {
	test("the version follows the name unless the name has it; at most 128 bytes", () => {
		expect(displayNameOf(choice())).toBe("Gemma-3-4B-it Q4_K_M");
		expect(
			displayNameOf(
				choice({ candidate: { ...candidate(), name: "Qwen3 8B Q4_K_M" } }),
			),
		).toBe("Qwen3 8B Q4_K_M");
		const long = displayNameOf(
			choice({ candidate: { ...candidate(), name: "ü".repeat(100) } }),
		);
		expect(new TextEncoder().encode(long).length).toBeLessThanOrEqual(128);
		expect(long).toBe("ü".repeat(64));
	});

	test("a name the device takes: no space left by the cut, no control characters, never empty", () => {
		const named = (name: string, label = "Q4_K_M") =>
			displayNameOf(
				choice({
					candidate: { ...candidate(), name },
					option: option({ label }),
				}),
			);
		expect(named("x".repeat(127))).toBe("x".repeat(127));
		expect(named("Qwen3\n8B")).toBe("Qwen3 8B Q4_K_M");
		expect(named("Qwen3 8B\u0085", "")).toBe("Qwen3 8B");
		expect(named(" \t", "")).toBe("huggingface:o/m@1");
	});

	test("ids are management ids, unique on the device", () => {
		expect(modelIdFor("Qwen3-8B Q4_K_M", new Set())).toBe("qwen3-8b-q4_k_m");
		expect(
			modelIdFor(
				"Qwen3-8B Q4_K_M",
				new Set(["qwen3-8b-q4_k_m", "qwen3-8b-q4_k_m-2"]),
			),
		).toBe("qwen3-8b-q4_k_m-3");
		expect(modelIdFor("模型", new Set())).toBe("model");
		expect(modelIdFor("x".repeat(200), new Set())).toHaveLength(120);
	});

	test.each([
		["Qwen3-8B-Q4_K_M.gguf", "Q4_K_M"],
		["gemma-3-4b-it-UD-Q4_K_XL.gguf", "UD_Q4_K_XL"],
		["BF16/model-BF16-00001-of-00002.gguf", "BF16"],
		["model.onnx", undefined],
	])("%s → %s", (name, quant) => {
		expect(quantizationOf(name)).toBe(quant);
	});
});

describe("Bits a device can host", () => {
	test.each([
		["a local GGUF chat model", bit({ file_name: "m.gguf" }), "llamacpp", true],
		[
			"an MLX model",
			bit({ parameters: { provider: { provider_name: "MLX" } } }),
			"mlx",
			true,
		],
		[
			"an ONNX embedding model",
			bit({ type: IBitTypes.Embedding, file_name: "model.onnx" }),
			"onnx",
			true,
		],
		[
			"a GGUF embedding model",
			bit({ type: IBitTypes.Embedding, file_name: "nomic.Q8_0.gguf" }),
			"llamacpp",
			true,
		],
		[
			"a hosted provider",
			bit({ parameters: { provider: { provider_name: "openai" } } }),
			undefined,
			false,
		],
		[
			"a speech model",
			bit({ type: IBitTypes.Tts, file_name: "voice.onnx" }),
			undefined,
			false,
		],
	] as const)("%s", (_, model, engine, hostable) => {
		expect(engineOfBit(model)).toBe(engine);
		expect(isHostableBit(model)).toBe(hostable);
	});
});

test("SystemOne hub Bits install as decisions with an optional projector", () => {
	const root = bit({
		type: IBitTypes.SystemOne,
		file_name: "laya.gguf",
		size: 1000,
		hash: "a".repeat(64),
	});
	const candidate = fromHubPack(root, []);
	expect(candidate.kinds).toEqual(["systemone"]);
	const selected = {
		candidate,
		option: candidate.options[0]!,
		kind: "systemone" as const,
	};
	expect(modelSpecOf(selected).kind).toBe("systemone");
	expect(modelSpecOf(selected).projector).toBeUndefined();
	const withProjector = choice({
		kind: "systemone",
		projector: "mmproj-BF16.gguf",
	});
	expect(modelSpecOf(withProjector).projector).toBe("mmproj-BF16.gguf");
});
