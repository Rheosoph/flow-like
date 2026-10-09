import { describe, expect, test } from "bun:test";
import { sha256 as streamingSha256 } from "@noble/hashes/sha2";
import { type IBit, IBitTypes } from "../../../../../lib/schema";
import {
	LFS,
	QWEN3_8B_CONFIG,
	REVISIONS,
	SMALL_FILES,
	fakeHuggingFace,
	sampleRepos,
} from "./fake-hugging-face";
import { modelSpecOf } from "./model-options";
import {
	type Fetcher,
	fingerprint,
	fingerprintAll,
	hubCandidate,
	huggingFaceCandidate,
	lfsDigests,
	repositoryFacts,
	userBitCandidate,
} from "./model-reads";

const sha256 = async (text: string) =>
	[
		...new Uint8Array(
			await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)),
		),
	]
		.map((byte) => byte.toString(16).padStart(2, "0"))
		.join("");

const pinned = (repo: string, revision: string, path: string) =>
	`https://huggingface.co/${repo}/resolve/${revision}/${path}?download=true`;

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
		parameters: {},
		...overrides,
	} as IBit;
}

describe("Hugging Face repositories", () => {
	test("a GGUF repository: every quantization, the recommended first, and its license", async () => {
		const candidate = await huggingFaceCandidate(
			"Qwen/Qwen3-8B-GGUF",
			fakeHuggingFace().fetch,
		);
		expect(candidate).toMatchObject({
			source: "huggingface",
			key: `huggingface:Qwen/Qwen3-8B-GGUF@${REVISIONS.qwenGguf}`,
			name: "Qwen3-8B",
			kinds: ["chat"],
			license: "apache-2.0",
		});
		expect(
			candidate.options.map((option) => [option.label, option.recommended]),
		).toEqual([
			["Q4_K_M", true],
			["BF16", false],
			["Q8_0", false],
		]);
		const bf16 = candidate.options[1];
		expect(bf16?.files.map((file) => file.file_name)).toEqual([
			"BF16/Qwen3-8B-BF16-00001-of-00002.gguf",
			"BF16/Qwen3-8B-BF16-00002-of-00002.gguf",
		]);
		expect(bf16?.blocked).toBeUndefined();
	});

	test("a GGUF version: its file with the LFS sha256 and pinned source, facts from the base model's config.json", async () => {
		const hub = fakeHuggingFace();
		const candidate = await huggingFaceCandidate(
			"Qwen/Qwen3-8B-GGUF",
			hub.fetch,
		);
		const [q4] = candidate.options;
		expect(q4?.files).toEqual([
			{
				file_name: "Qwen3-8B-Q4_K_M.gguf",
				size: 5_027_784_064,
				digest: { algorithm: "sha256", hex: LFS.q4 },
				sources: [
					pinned(
						"Qwen/Qwen3-8B-GGUF",
						REVISIONS.qwenGguf,
						"Qwen3-8B-Q4_K_M.gguf",
					),
				],
				weights: true,
			},
		]);
		expect(q4?.facts).toEqual({
			params: 8_190_735_360,
			contextLength: 40_960,
			layers: 36,
			kvBytesPerToken: 147_456,
			bitsPerWeight: 4.89,
		});
		expect(hub.requests).toContain(
			"https://huggingface.co/Qwen/Qwen3-8B/resolve/main/config.json",
		);
	});

	test("an MLX repository: one version; small files without LFS get fingerprinted before the install", async () => {
		const hub = fakeHuggingFace();
		const candidate = await huggingFaceCandidate(
			"https://huggingface.co/mlx-community/Qwen3-4B-4bit",
			hub.fetch,
		);
		expect(candidate.kinds).toEqual(["chat"]);
		const [option] = candidate.options;
		expect(option).toMatchObject({
			id: "mlx",
			label: "MLX 4-bit",
			engine: "mlx",
		});
		expect(option?.facts).toMatchObject({ bitsPerWeight: 4.5, layers: 36 });
		expect(
			option?.files.map((file) => [
				file.file_name,
				file.digest?.hex ?? null,
				file.weights,
			]),
		).toEqual([
			["config.json", null, false],
			["model.safetensors", LFS.mlxWeights, true],
			["tokenizer.json", LFS.mlxTokenizer, false],
			["tokenizer_config.json", null, false],
		]);
		if (!option) throw new Error("The MLX repository has no version.");
		const choice = { candidate, option, kind: "chat" as const };
		const digests = await fingerprintAll(
			option.files.filter((file) => !file.digest),
			hub.fetch,
		);
		const spec = modelSpecOf(choice, digests);
		expect(spec.assets[0]?.digest).toEqual({
			algorithm: "sha256",
			hex: await sha256(SMALL_FILES.mlxConfig),
		});
		expect(spec.assets[3]?.digest).toEqual({
			algorithm: "sha256",
			hex: await sha256(SMALL_FILES.mlxTokenizerConfig),
		});
	});

	test("an embedding repository the import can't classify offers Embedding first, from its Hub task", async () => {
		const candidate = await huggingFaceCandidate(
			"nomic-ai/nomic-embed-text-v1.5-GGUF",
			fakeHuggingFace().fetch,
		);
		expect(candidate.kinds).toEqual(["embedding", "chat"]);
		expect(candidate.options[0]?.facts).toEqual({ bitsPerWeight: 8.5 });
	});

	test("an embedding repository whose config.json names a causal LM still offers Embedding first", async () => {
		const config = JSON.stringify({
			...QWEN3_8B_CONFIG,
			hidden_size: 1024,
			num_hidden_layers: 28,
			num_attention_heads: 16,
		});
		const repos = {
			...sampleRepos(),
			"Qwen/Qwen3-Embedding-0.6B-GGUF": {
				revision: "5".repeat(40),
				info: { pipeline_tag: "feature-extraction", tags: ["gguf"] },
				files: {
					"config.json": { size: config.length, body: config },
					"Qwen3-Embedding-0.6B-Q8_0.gguf": {
						size: 639_000_000,
						lfs: "c2".repeat(32),
					},
				},
			},
		};
		const candidate = await huggingFaceCandidate(
			"Qwen/Qwen3-Embedding-0.6B-GGUF",
			fakeHuggingFace(repos).fetch,
		);
		expect(candidate.kinds).toEqual(["embedding", "chat"]);
	});

	test("a missing repository fails with the import's reason", async () => {
		await expect(
			huggingFaceCandidate("nobody/nothing", fakeHuggingFace().fetch),
		).rejects.toThrow("Hugging Face model was not found");
	});

	test("facts that can't be read stay unknown", async () => {
		expect(
			await repositoryFacts(
				"nobody/nothing",
				REVISIONS.qwenGguf,
				fakeHuggingFace().fetch,
			),
		).toEqual({});
	});
});

describe("Git LFS digests", () => {
	test("each pinned link's sha256 from its folder at the commit, across pages; other links have none", async () => {
		const hub = fakeHuggingFace();
		const q4 = pinned(
			"Qwen/Qwen3-8B-GGUF",
			REVISIONS.qwenGguf,
			"Qwen3-8B-Q4_K_M.gguf",
		);
		const q8 = pinned(
			"Qwen/Qwen3-8B-GGUF",
			REVISIONS.qwenGguf,
			"Qwen3-8B-Q8_0.gguf",
		);
		const split = pinned(
			"Qwen/Qwen3-8B-GGUF",
			REVISIONS.qwenGguf,
			"BF16/Qwen3-8B-BF16-00002-of-00002.gguf",
		);
		const digests = await lfsDigests(
			[q4, q8, split, "https://cdn.flow-like.com/bits/model.gguf"],
			hub.fetch,
		);
		expect(digests).toEqual(
			new Map([
				[q4, LFS.q4],
				[q8, LFS.q8],
				[split, LFS.bf16b],
			]),
		);
		expect(hub.requests.some((url) => url.includes("cursor=2"))).toBe(true);
		expect(hub.requests).toContain(
			`https://huggingface.co/api/models/Qwen/Qwen3-8B-GGUF/tree/${REVISIONS.qwenGguf}/BF16`,
		);
	});

	test("a commit the repository doesn't have gives nothing", async () => {
		const stale = pinned(
			"Qwen/Qwen3-8B-GGUF",
			"9".repeat(40),
			"Qwen3-8B-Q4_K_M.gguf",
		);
		expect(await lfsDigests([stale], fakeHuggingFace().fetch)).toEqual(
			new Map(),
		);
	});

	test("a listing that fails otherwise fails the lookup with why, never as a file without a fingerprint", async () => {
		const hub = fakeHuggingFace();
		const busy: Fetcher = async (input, init) =>
			input.includes("/tree/")
				? new Response("Too Many Requests", { status: 429 })
				: hub.fetch(input, init);
		const offline: Fetcher = async () => {
			throw new TypeError("Failed to fetch");
		};
		const link = pinned(
			"Qwen/Qwen3-8B-GGUF",
			REVISIONS.qwenGguf,
			"Qwen3-8B-Q8_0.gguf",
		);
		await expect(lfsDigests([link], busy)).rejects.toThrow(
			`Reading the Git LFS fingerprints of Qwen/Qwen3-8B-GGUF at ${REVISIONS.qwenGguf} from Hugging Face failed: HTTP 429. Try again in a moment.`,
		);
		await expect(lfsDigests([link], offline)).rejects.toThrow(
			"from Hugging Face failed: Failed to fetch. Try again in a moment.",
		);
		await expect(
			userBitCandidate(
				bit({
					id: "my-qwen",
					file_name: "Qwen3-8B-Q8_0.gguf",
					download_link: link,
					size: 8_709_519_168,
				}),
				busy,
			),
		).rejects.toThrow("failed: HTTP 429.");
	});
});

describe("fingerprints", () => {
	const config = {
		file_name: "config.json",
		size: new TextEncoder().encode(SMALL_FILES.mlxConfig).length,
		sources: [
			"https://cdn.flow-like.com/missing/config.json",
			pinned("mlx-community/Qwen3-4B-4bit", REVISIONS.mlx, "config.json"),
		],
		weights: false,
	};

	test("the sha256 of the first source that answers with the expected bytes", async () => {
		expect(await fingerprint(config, fakeHuggingFace().fetch)).toEqual({
			algorithm: "sha256",
			hex: await sha256(SMALL_FILES.mlxConfig),
		});
	});

	test("wrong sizes and errors say what failed", async () => {
		const hub = fakeHuggingFace();
		await expect(
			fingerprint(
				{ ...config, size: 12, sources: config.sources.slice(1) },
				hub.fetch,
			),
		).rejects.toThrow(
			`config.json from huggingface.co has ${config.size} bytes; 12 were expected.`,
		);
		await expect(
			fingerprint(
				{
					...config,
					sources: [
						pinned(
							"mlx-community/Qwen3-4B-4bit",
							REVISIONS.mlx,
							"missing.json",
						),
					],
				},
				hub.fetch,
			),
		).rejects.toThrow(
			"Downloading config.json from huggingface.co to fingerprint it failed with HTTP 404.",
		);
		await expect(
			fingerprint({ ...config, size: Number.MAX_SAFE_INTEGER + 1 }, hub.fetch),
		).rejects.toThrow("invalid size");
		await expect(
			fingerprint({ ...config, sources: [] }, hub.fetch),
		).rejects.toThrow("config.json has no source to fingerprint it from.");
	});

	test("fingerprints files above 64 MiB as a stream without buffering the whole file", async () => {
		const chunk = new Uint8Array(1024 * 1024).fill(17);
		const expected = streamingSha256.create();
		let chunks = 0;
		const fetcher: Fetcher = async () => {
			const response = new Response(
				new ReadableStream({
					pull(controller) {
						if (chunks === 65) return controller.close();
						chunks += 1;
						expected.update(chunk);
						controller.enqueue(chunk);
					},
				}),
			);
			response.arrayBuffer = async () => {
				throw new Error("Do not buffer the entire model file.");
			};
			return response;
		};
		const result = await fingerprint(
			{ ...config, size: 65 * chunk.length },
			fetcher,
		);
		expect(result).toEqual({
			algorithm: "sha256",
			hex: Array.from(expected.digest(), (byte) =>
				byte.toString(16).padStart(2, "0"),
			).join(""),
		});
		expect(chunks).toBe(65);
	});

	test("a stopped run tries no other source or file and returns no digests", async () => {
		const hub = fakeHuggingFace();
		const run = new AbortController();
		const stopping: Fetcher = async (input, init) => {
			run.abort();
			return hub.fetch(input, init);
		};
		const counted: number[] = [];
		await expect(
			fingerprintAll([config, config], stopping, {
				signal: run.signal,
				onFile: (done) => counted.push(done),
			}),
		).rejects.toMatchObject({ name: "AbortError" });
		expect(hub.requests).toEqual([config.sources[0]]);
		expect(counted).toEqual([]);
	});
});

describe("Bits", () => {
	const qwenOrigin = {
		provider_name: "Local",
		model_id: "Qwen/Qwen3-8B-GGUF",
		version: REVISIONS.qwenGguf,
	};

	test("a hub model: blake3 digests from the hub, its CDN link first, then the pinned Hugging Face file", async () => {
		const root = bit({
			id: "qwen3-8b",
			type: IBitTypes.Vlm,
			hash: "c".repeat(64),
			file_name: "Qwen3-8B-Q4_K_M.gguf",
			download_link: "https://cdn.flow-like.com/bits/qwen3-8b",
			size: 5_027_784_064,
			meta: { en: { name: "Qwen3 8B" } as IBit["meta"][string] },
			parameters: { context_length: 32_768, provider: qwenOrigin },
		});
		const projector = bit({
			id: "qwen3-8b-mmproj",
			type: IBitTypes.Projection,
			hash: "d".repeat(64),
			file_name: "mmproj-F16.gguf",
			download_link: "https://cdn.flow-like.com/bits/mmproj",
			size: 851_251_104,
		});
		const candidate = await hubCandidate(
			root,
			{ getPackFromBit: async () => ({ bits: [root, projector] }) },
			fakeHuggingFace().fetch,
		);
		expect(candidate).toMatchObject({
			source: "hub",
			key: "hub:qwen3-8b",
			name: "Qwen3 8B",
			kinds: ["vision", "chat"],
		});
		const [option] = candidate.options;
		expect(option?.files).toEqual([
			{
				file_name: "Qwen3-8B-Q4_K_M.gguf",
				size: 5_027_784_064,
				digest: { algorithm: "blake3", hex: "c".repeat(64) },
				sources: [
					"https://cdn.flow-like.com/bits/qwen3-8b",
					pinned(
						"Qwen/Qwen3-8B-GGUF",
						REVISIONS.qwenGguf,
						"Qwen3-8B-Q4_K_M.gguf",
					),
				],
				weights: true,
			},
		]);
		expect(option?.projectors.map((file) => file.file_name)).toEqual([
			"mmproj-F16.gguf",
		]);
		expect(option?.facts).toMatchObject({
			contextLength: 40_960,
			kvBytesPerToken: QWEN3_8B_CONFIG.num_hidden_layers * 8 * 128 * 4,
			bitsPerWeight: 4.89,
		});
	});

	test("a user GGUF Bit: its pinned file's LFS sha256", async () => {
		const link = pinned(
			"Qwen/Qwen3-8B-GGUF",
			REVISIONS.qwenGguf,
			"Qwen3-8B-Q8_0.gguf",
		);
		const candidate = await userBitCandidate(
			bit({
				id: "my-qwen",
				hash: "user-source-0123",
				file_name: "Qwen3-8B-Q8_0.gguf",
				download_link: link,
				size: 8_709_519_168,
				parameters: {
					context_length: 40_960,
					provider: { ...qwenOrigin, params: {} },
				},
			}),
			fakeHuggingFace().fetch,
		);
		expect(candidate.key).toBe("bits:my-qwen");
		expect(candidate.options[0]?.files[0]?.digest).toEqual({
			algorithm: "sha256",
			hex: LFS.q8,
		});
		expect(candidate.options[0]?.label).toBe("Q8_0");
	});
});
