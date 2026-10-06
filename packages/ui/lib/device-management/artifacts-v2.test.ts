import { describe, expect, test } from "bun:test";
import { sha256 } from "@noble/hashes/sha2";
import {
	type PackagedBitAsset,
	type PackagedBitMetadata,
	prepareProjectArtifact,
} from "./artifacts";

const encoder = new TextEncoder();
const hex = (bytes: Uint8Array) =>
	Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");

function asset(
	bitId: string,
	fill: string,
	fileName: string,
	size: number,
): PackagedBitAsset {
	return {
		bit_id: bitId,
		descriptor: {
			digest: { algorithm: "blake3", hex: fill.repeat(64) },
			size,
			file_name: fileName,
			sources: ["https://cdn.flow-like.com/bits/model"],
		},
	};
}

const TOKENIZER = "tokens";

/** Weights as a model-store asset, the tokenizer as an artifact file (the Rust `v2()` fixture). */
function v2(): Extract<PackagedBitMetadata, { version: 2 }> {
	return {
		version: 2,
		bit: {
			id: "model",
			hash: "weights-hash",
			file_name: "model.gguf",
			size: 4096,
			dependencies: ["tokenizer"],
		},
		dependencies: [
			{
				id: "tokenizer",
				hash: "tokenizer-hash",
				file_name: "tokenizer.json",
				size: TOKENIZER.length,
			},
		],
		assets: [asset("model", "a", "model.gguf", 4096)],
		artifacts: [
			{
				path: "bits/tokenizer-hash/tokenizer.json",
				size: TOKENIZER.length,
				sha256: hex(sha256(encoder.encode(TOKENIZER))),
			},
		],
	};
}

function inputs(
	metadata: unknown,
	extra: { path: string; text: string }[] = [],
) {
	const bytes = encoder.encode(JSON.stringify(metadata));
	return {
		pins: {
			bit_pins: [{ bit_id: "model", metadata_sha256: hex(sha256(bytes)) }],
			package_pins: [],
		},
		files: [
			{ path: "apps/project/manifest.app", file: new Blob(["project"]) },
			{ path: "bits/metadata/model.json", file: new Blob([bytes]) },
			...extra.map(({ path, text }) => ({ path, file: new Blob([text]) })),
		],
	};
}

const tokenizerFile = {
	path: "bits/tokenizer-hash/tokenizer.json",
	text: TOKENIZER,
};

function prepare(metadata: unknown, extra = [tokenizerFile]) {
	const { pins, files } = inputs(metadata, extra);
	return prepareProjectArtifact("project", files, undefined, pins);
}

describe("Bit metadata v2", () => {
	test("names model-store assets the artifact never carries", async () => {
		const result = await prepare(v2());
		const manifest = JSON.parse(new TextDecoder().decode(result.manifest));
		expect(manifest.files.map((file: { path: string }) => file.path)).toEqual([
			"apps/project/manifest.app",
			"bits/metadata/model.json",
			"bits/tokenizer-hash/tokenizer.json",
		]);
		expect(result.models?.pins.map((pin) => pin.bit_id)).toEqual(["model"]);
		expect(result.models?.assets).toEqual([
			{
				pin: "model",
				bitId: "model",
				bitHash: "weights-hash",
				descriptor: asset("model", "a", "model.gguf", 4096).descriptor,
			},
		]);
		await expect(
			prepare(v2(), [
				tokenizerFile,
				{ path: "bits/weights-hash/model.gguf", text: "weights" },
			]),
		).rejects.toThrow("unselected");
	});

	test("v1 metadata names no model assets", async () => {
		const { assets: _, version: __, artifacts = [], ...rest } = v2();
		const v1 = {
			...rest,
			artifacts: [
				...artifacts,
				{
					path: "bits/weights-hash/model.gguf",
					size: 7,
					sha256: hex(sha256(encoder.encode("weights"))),
				},
			],
		};
		v1.bit = { ...v1.bit, size: 7 };
		const result = await prepare(v1, [
			tokenizerFile,
			{ path: "bits/weights-hash/model.gguf", text: "weights" },
		]);
		expect(result.models).toBeUndefined();
	});

	test("every file may be a model asset, and dependencies may be left out", async () => {
		const stored = v2();
		stored.artifacts = [];
		stored.assets.push(asset("tokenizer", "c", "tokenizer.json", 6));
		const result = await prepare(stored, []);
		expect(result.models?.assets.map((entry) => entry.bitId)).toEqual([
			"model",
			"tokenizer",
		]);
		const bare = v2();
		bare.dependencies = undefined;
		bare.artifacts = undefined;
		bare.bit = { ...bare.bit, dependencies: [] };
		expect((await prepare(bare, [])).models?.assets).toHaveLength(1);
	});

	test("split parts run in load order, the first named like the Bit", async () => {
		const split = v2();
		split.bit = {
			...split.bit,
			file_name: "qwen-00001-of-00002.gguf",
			size: null,
		};
		split.assets = [
			asset("model", "a", "qwen-00001-of-00002.gguf", 4096),
			asset("model", "b", "qwen-00002-of-00002.gguf", 2048),
		];
		expect((await prepare(split)).models?.assets).toHaveLength(2);
		const swapped = { ...split, assets: [...split.assets].reverse() };
		await expect(prepare(swapped)).rejects.toThrow("another file name");
		const twin = {
			...split,
			assets: [
				split.assets[0],
				{ ...split.assets[1], descriptor: split.assets[0].descriptor },
			],
		};
		await expect(prepare(twin)).rejects.toThrow("twice");
	});

	test("Bits that share a file share one asset", async () => {
		const shared = v2();
		shared.dependencies = [
			...(shared.dependencies ?? []),
			{ id: "copy", hash: "copy-hash", file_name: "model.gguf", size: 4096 },
		];
		shared.assets.push(asset("copy", "a", "model.gguf", 4096));
		expect((await prepare(shared)).models?.assets).toHaveLength(1);
	});

	const refusals: [string, (metadata: ReturnType<typeof v2>) => unknown][] = [
		[
			"a single asset of another size than its Bit",
			(metadata) => {
				metadata.assets[0].descriptor.size = 4097;
				return metadata;
			},
		],
		[
			"an asset of an unknown Bit",
			(metadata) => {
				metadata.assets.push(asset("missing", "d", "model.gguf", 4096));
				return metadata;
			},
		],
		["a file listed nowhere", (metadata) => ({ ...metadata, artifacts: [] })],
		[
			"a file listed as an asset and as an artifact",
			(metadata) => ({
				...metadata,
				artifacts: [
					...(metadata.artifacts ?? []),
					{
						path: "bits/weights-hash/model.gguf",
						size: 4096,
						sha256: "a".repeat(64),
					},
				],
			}),
		],
		[
			"an id two file-backed Bits share",
			(metadata) => ({
				...metadata,
				dependencies: [
					...(metadata.dependencies ?? []),
					{ id: "model", hash: "x", file_name: "model.gguf" },
				],
			}),
		],
		[
			"a plain HTTP source",
			(metadata) => {
				metadata.assets[0].descriptor.sources = ["http://cdn.flow-like.com/m"];
				return metadata;
			},
		],
		[
			"a source with credentials",
			(metadata) => {
				metadata.assets[0].descriptor.sources = [
					"https://user:secret@cdn.flow-like.com/m",
				];
				return metadata;
			},
		],
		[
			"nine sources",
			(metadata) => {
				metadata.assets[0].descriptor.sources = Array.from(
					{ length: 9 },
					(_, index) => `https://cdn.flow-like.com/${index}`,
				);
				return metadata;
			},
		],
		[
			"an uppercase digest",
			(metadata) => {
				metadata.assets[0].descriptor.digest.hex = "A".repeat(64);
				return metadata;
			},
		],
		[
			"a traversal in a file name",
			(metadata) => {
				metadata.bit = { ...metadata.bit, file_name: "../model.gguf" };
				metadata.assets[0].descriptor.file_name = "../model.gguf";
				return metadata;
			},
		],
		["an unknown field", (metadata) => ({ ...metadata, extra: 1 })],
		[
			"an unknown descriptor field",
			(metadata) => {
				(
					metadata.assets[0].descriptor as unknown as Record<string, unknown>
				).etag = "x";
				return metadata;
			},
		],
		["version 3", (metadata) => ({ ...metadata, version: 3 })],
		["a missing asset list", ({ assets: _, ...metadata }) => metadata],
		[
			"another root",
			(metadata) => ({ ...metadata, bit: { ...metadata.bit, id: "other" } }),
		],
		[
			"a v1 wrapper with assets",
			({ version: _, ...metadata }) => ({ ...metadata, dependencies: [] }),
		],
	];
	for (const [name, change] of refusals)
		test(`refuses ${name}`, async () => {
			await expect(prepare(change(v2()))).rejects.toThrow();
		});
});
