/*
 * An in-memory Hugging Face for the add-model tests: the model API, the
 * repository tree (recursive or one folder, two entries per page) and
 * `resolve` downloads of small files. Large files exist only in the tree.
 *
 *   const hub = fakeHuggingFace();
 *   await huggingFaceCandidate("Qwen/Qwen3-8B-GGUF", hub.fetch);
 *   hub.requests  // every URL, oldest first
 */

export const REVISIONS = {
	qwenGguf: "1".repeat(40),
	qwenBase: "2".repeat(40),
	mlx: "3".repeat(40),
	embed: "4".repeat(40),
} as const;

export const LFS = {
	q4: "a1".repeat(32),
	q8: "a2".repeat(32),
	bf16a: "a3".repeat(32),
	bf16b: "a4".repeat(32),
	mlxWeights: "b1".repeat(32),
	mlxTokenizer: "b2".repeat(32),
	embed: "c1".repeat(32),
} as const;

export const QWEN3_8B_CONFIG = {
	architectures: ["Qwen3ForCausalLM"],
	hidden_size: 4096,
	num_hidden_layers: 36,
	num_attention_heads: 32,
	num_key_value_heads: 8,
	head_dim: 128,
	max_position_embeddings: 40_960,
};

const QWEN3_4B_MLX_CONFIG = {
	architectures: ["Qwen3ForCausalLM"],
	hidden_size: 2560,
	num_hidden_layers: 36,
	num_attention_heads: 32,
	num_key_value_heads: 8,
	head_dim: 128,
	max_position_embeddings: 40_960,
	quantization: { group_size: 64, bits: 4 },
};

interface FakeFile {
	size: number;
	lfs?: string;
	/** Bytes `resolve` answers with; large files have none. */
	body?: string;
}

interface FakeRepo {
	revision: string;
	info: Record<string, unknown>;
	files: Record<string, FakeFile>;
}

const small = (body: string): FakeFile => ({
	size: new TextEncoder().encode(body).length,
	body,
});

const json = (value: unknown) => small(JSON.stringify(value));

export const SMALL_FILES = {
	mlxConfig: JSON.stringify(QWEN3_4B_MLX_CONFIG),
	mlxTokenizerConfig: JSON.stringify({ model_max_length: 40_960 }),
};

export function sampleRepos(): Record<string, FakeRepo> {
	return {
		"Qwen/Qwen3-8B-GGUF": {
			revision: REVISIONS.qwenGguf,
			info: {
				pipeline_tag: "text-generation",
				tags: ["gguf", "license:apache-2.0"],
				cardData: { license: "apache-2.0", base_model: "Qwen/Qwen3-8B" },
				gguf: { total: 8_190_735_360, context_length: 40_960 },
			},
			files: {
				"README.md": small("# Qwen3-8B GGUF"),
				"Qwen3-8B-Q4_K_M.gguf": { size: 5_027_784_064, lfs: LFS.q4 },
				"Qwen3-8B-Q8_0.gguf": { size: 8_709_519_168, lfs: LFS.q8 },
				"BF16/Qwen3-8B-BF16-00001-of-00002.gguf": {
					size: 9_000_000_000,
					lfs: LFS.bf16a,
				},
				"BF16/Qwen3-8B-BF16-00002-of-00002.gguf": {
					size: 7_400_000_000,
					lfs: LFS.bf16b,
				},
			},
		},
		"Qwen/Qwen3-8B": {
			revision: REVISIONS.qwenBase,
			info: { pipeline_tag: "text-generation", tags: ["safetensors"] },
			files: { "config.json": json(QWEN3_8B_CONFIG) },
		},
		"mlx-community/Qwen3-4B-4bit": {
			revision: REVISIONS.mlx,
			info: {
				library_name: "mlx",
				pipeline_tag: "text-generation",
				tags: ["mlx", "license:apache-2.0"],
				cardData: { license: "apache-2.0" },
			},
			files: {
				"config.json": small(SMALL_FILES.mlxConfig),
				"model.safetensors": { size: 2_262_000_000, lfs: LFS.mlxWeights },
				"tokenizer.json": { size: 11_422_654, lfs: LFS.mlxTokenizer },
				"tokenizer_config.json": small(SMALL_FILES.mlxTokenizerConfig),
			},
		},
		"nomic-ai/nomic-embed-text-v1.5-GGUF": {
			revision: REVISIONS.embed,
			info: { pipeline_tag: "sentence-similarity", tags: ["gguf"] },
			files: {
				"nomic-embed-text-v1.5.Q8_0.gguf": {
					size: 146_146_432,
					lfs: LFS.embed,
				},
			},
		},
	};
}

const PAGE = 2;

function entry(path: string, file: FakeFile) {
	return {
		type: "file",
		path,
		size: file.size,
		oid: `git-${path.length}`,
		...(file.lfs
			? { lfs: { oid: file.lfs, size: file.size, pointerSize: 134 } }
			: {}),
	};
}

function answer(value: unknown, headers: Record<string, string> = {}) {
	return new Response(JSON.stringify(value), {
		status: 200,
		headers: { "content-type": "application/json", ...headers },
	});
}

const missing = () => new Response("Entry not found", { status: 404 });

export interface FakeHuggingFace {
	fetch(input: string, init?: RequestInit): Promise<Response>;
	requests: string[];
	repos: Record<string, FakeRepo>;
}

function treeOf(repo: FakeRepo, folder: string, recursive: boolean) {
	const prefix = folder ? `${folder}/` : "";
	return Object.entries(repo.files)
		.filter(([path]) => {
			if (!path.startsWith(prefix)) return false;
			return recursive || !path.slice(prefix.length).includes("/");
		})
		.map(([path, file]) => entry(path, file));
}

function treePage(url: URL, repo: FakeRepo, folder: string) {
	const entries = treeOf(
		repo,
		folder,
		url.searchParams.get("recursive") === "true",
	);
	const cursor = Number(url.searchParams.get("cursor") ?? "0");
	const next = cursor + PAGE;
	if (next >= entries.length) return answer(entries.slice(cursor));
	const link = new URL(url);
	link.searchParams.set("cursor", String(next));
	return answer(entries.slice(cursor, next), {
		link: `<${link.toString()}>; rel="next"`,
	});
}

function api(url: URL, repos: Record<string, FakeRepo>) {
	const [, , , owner = "", name = "", route, revision, ...rest] =
		url.pathname.split("/");
	const id = `${owner}/${name}`;
	const repo = repos[id];
	if (!repo) return missing();
	if (route === "tree") {
		if (revision !== repo.revision) return missing();
		return treePage(url, repo, rest.map(decodeURIComponent).join("/"));
	}
	return answer({
		id,
		author: owner,
		sha: repo.revision,
		private: false,
		gated: false,
		...repo.info,
	});
}

function resolve(url: URL, repos: Record<string, FakeRepo>) {
	const [, owner = "", name = "", , revision = "", ...path] =
		url.pathname.split("/");
	const repo = repos[`${owner}/${name}`];
	const file = repo?.files[path.map(decodeURIComponent).join("/")];
	const pinned = revision === repo?.revision || revision === "main";
	if (!file?.body || !pinned) return missing();
	return new Response(file.body, { status: 200 });
}

export function fakeHuggingFace(
	repos: Record<string, FakeRepo> = sampleRepos(),
): FakeHuggingFace {
	const requests: string[] = [];
	return {
		requests,
		repos,
		fetch: async (input) => {
			requests.push(input);
			const url = new URL(input);
			if (url.origin !== "https://huggingface.co")
				throw new TypeError(`Failed to fetch ${input}`);
			return url.pathname.startsWith("/api/models/")
				? api(url, repos)
				: resolve(url, repos);
		},
	};
}
