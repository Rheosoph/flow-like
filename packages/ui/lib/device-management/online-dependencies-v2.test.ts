import { afterEach, describe, expect, test } from "bun:test";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { IApp } from "../schema/app/app";
import type { IBit } from "../schema/bit/bit";
import { prepareOnlineDependencies } from "./online-dependencies";

const profile = { id: "profile" } as IProfile;
const REVISION = "03e404fd168941cfed98f46654680130dd85968b";
const BLAKE3 =
	"9a129038d9a00aed0cf6a7ea059ca50a813449061ab87848cf1a13eafdf33b2c";
const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});

function hubModel(overrides: Partial<IBit> = {}): IBit {
	return {
		id: "qwen3-8b",
		hub: "hub.test",
		type: "Llm",
		hash: BLAKE3,
		file_name: "Qwen3-8B-Q4_K_M.gguf",
		size: 4_920_734_016,
		download_link: "https://cdn.flow-like.com/bits/qwen3-8b-q4",
		dependencies: [],
		parameters: {
			provider: {
				provider_name: "Local",
				model_id: "Qwen/Qwen3-8B-GGUF",
				version: REVISION,
			},
		},
		...overrides,
	} as IBit;
}

function backendWith(bit: IBit, dependencies: IBit[] = []) {
	const app = {
		id: "project",
		bits: [bit.id],
		packages: {},
	} as unknown as IApp;
	const backend = {
		bitState: { getBit: async () => bit },
		apiState: {
			get: async (_: unknown, path: string) =>
				path === `bit/${bit.id}/dependencies`
					? dependencies
					: { version: 1, project_id: app.id, documents: { app } },
		},
	} as unknown as IBackendState;
	return { app, backend };
}

/** Every download answers `bytes`; the URLs asked for are recorded. */
function serve(bytes: Uint8Array<ArrayBuffer>) {
	const asked: string[] = [];
	globalThis.fetch = (async (input: RequestInfo | URL) => {
		asked.push(String(input));
		return new Response(bytes);
	}) as typeof fetch;
	return asked;
}

function serveLargeFile(size: number) {
	const file = new Blob([]);
	Object.defineProperty(file, "size", { value: size });
	file.arrayBuffer = async () => {
		throw new Error("Read model files in chunks.");
	};
	file.slice = (start = 0, end = size) => {
		expect(end - start).toBeLessThanOrEqual(1024 * 1024);
		return new Blob([]);
	};
	const asked: string[] = [];
	globalThis.fetch = (async (input: RequestInfo | URL) => {
		asked.push(String(input));
		const response = new Response(new Uint8Array(), {
			headers: { "content-length": String(size) },
		});
		response.blob = async () => file;
		return response;
	}) as typeof fetch;
	return asked;
}

async function metadataOf(
	exported: Awaited<ReturnType<typeof prepareOnlineDependencies>>,
	bitId: string,
) {
	const file = exported.artifact.files.find(
		(entry) => entry.path === `bits/metadata/${bitId}.json`,
	);
	if (!file) throw new Error(`metadata of ${bitId} is missing`);
	return JSON.parse(new TextDecoder().decode(await file.file.arrayBuffer()));
}

describe("online export for devices with a model store", () => {
	test("model-store assets above 64 GiB keep their exact size without downloading", async () => {
		const size = 65 * 1024 ** 3;
		const asked = serve(new Uint8Array());
		const { app, backend } = backendWith(hubModel({ size }));
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			profile,
			undefined,
			undefined,
			true,
		);
		expect(asked).toEqual([]);
		expect(exported.artifact.models?.assets[0]?.descriptor.size).toBe(size);
	});
	test("a file with a digest becomes an asset with its sources and is never downloaded", async () => {
		const asked = serve(new Uint8Array());
		const { app, backend } = backendWith(hubModel());
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			profile,
			undefined,
			undefined,
			true,
		);
		expect(asked).toEqual([]);
		const metadata = await metadataOf(exported, "qwen3-8b");
		expect(metadata.version).toBe(2);
		expect(metadata.artifacts).toBeUndefined();
		expect(metadata.assets).toEqual([
			{
				bit_id: "qwen3-8b",
				descriptor: {
					digest: { algorithm: "blake3", hex: BLAKE3 },
					size: 4_920_734_016,
					file_name: "Qwen3-8B-Q4_K_M.gguf",
					sources: [
						"https://cdn.flow-like.com/bits/qwen3-8b-q4",
						`https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/${REVISION}/Qwen3-8B-Q4_K_M.gguf?download=true`,
					],
				},
			},
		]);
		expect(JSON.stringify(metadata.bit)).not.toContain("cdn.flow-like.com");
		expect(
			exported.artifact.files.some((file) =>
				file.path.startsWith(`bits/${BLAKE3}`),
			),
		).toBe(false);
		expect(exported.artifact.models?.assets).toHaveLength(1);
		expect(exported.artifact.models?.pins).toEqual(exported.assets.bit_pins);
	});

	test("a small file without a digest still travels in the artifact", async () => {
		const tokenizer = new TextEncoder().encode("tokens");
		const asked = serve(tokenizer);
		const root = hubModel({ dependencies: ["tokenizer"] });
		const dependency = {
			id: "tokenizer",
			hub: "hub.test",
			type: "Tokenizer",
			hash: "tokenizer",
			file_name: "tokenizer.json",
			size: tokenizer.length,
			download_link: "https://cdn.flow-like.com/bits/tokenizer",
			dependencies: [],
			parameters: {},
		} as unknown as IBit;
		const { app, backend } = backendWith(root, [dependency]);
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			profile,
			undefined,
			undefined,
			true,
		);
		expect(asked).toEqual(["https://cdn.flow-like.com/bits/tokenizer"]);
		const metadata = await metadataOf(exported, "qwen3-8b");
		expect(metadata.version).toBe(2);
		expect(
			metadata.assets.map((asset: { bit_id: string }) => asset.bit_id),
		).toEqual(["qwen3-8b"]);
		expect(metadata.artifacts).toEqual([
			expect.objectContaining({
				path: "bits/tokenizer/tokenizer.json",
				size: tokenizer.length,
			}),
		]);
	});

	test("a large file without a digest is downloaded and fingerprinted in chunks", async () => {
		const size = 257 * 1024 ** 2;
		const asked = serveLargeFile(size);
		const { app, backend } = backendWith(
			hubModel({ hash: "user-source-abc", id: "my-model", size }),
		);
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			profile,
			undefined,
			undefined,
			true,
		);
		expect(asked).toHaveLength(1);
		expect(exported.artifact.descriptor.total_bytes).toBeGreaterThan(size);
		expect((await metadataOf(exported, "my-model")).artifacts[0].size).toBe(
			size,
		);
	});

	test("an asset above 4 GiB whose only source is signed travels in the artifact", async () => {
		const size = 5 * 1024 ** 3;
		const asked = serveLargeFile(size);
		const { app, backend } = backendWith(
			hubModel({
				size,
				download_link: "https://cdn.flow-like.com/bits/q4?Signature=private",
				parameters: { provider: { provider_name: "Local" } },
			}),
		);
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			profile,
			undefined,
			undefined,
			true,
		);
		expect(asked).toEqual([
			"https://cdn.flow-like.com/bits/q4?Signature=private",
		]);
		const metadata = await metadataOf(exported, "qwen3-8b");
		expect(metadata.artifacts[0].size).toBe(size);
		expect(JSON.stringify(metadata)).not.toContain("Signature");
	});

	test("a small file whose only link is signed travels in the artifact, as without a model store", async () => {
		const tokenizer = new TextEncoder().encode("tokens");
		const signed =
			"https://cdn.flow-like.com/bits/tokenizer?X-Amz-Expires=86400&X-Amz-Signature=private";
		const asked = serve(tokenizer);
		const root = hubModel({
			dependencies: ["tokenizer"],
			parameters: { provider: { provider_name: "Local" } },
		});
		const dependency = {
			id: "tokenizer",
			hub: "hub.test",
			type: "Tokenizer",
			hash: "b".repeat(64),
			file_name: "tokenizer.json",
			size: tokenizer.length,
			download_link: signed,
			dependencies: [],
			parameters: {},
		} as unknown as IBit;
		const { app, backend } = backendWith(root, [dependency]);
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			profile,
			undefined,
			undefined,
			true,
		);
		expect(asked).toEqual([signed]);
		const metadata = await metadataOf(exported, "qwen3-8b");
		expect(
			metadata.assets.map((asset: { bit_id: string }) => asset.bit_id),
		).toEqual(["qwen3-8b"]);
		expect(metadata.artifacts).toEqual([
			expect.objectContaining({
				path: `bits/${"b".repeat(64)}/tokenizer.json`,
				size: tokenizer.length,
			}),
		]);
		expect(JSON.stringify(metadata)).not.toContain("Signature");
	});

	test("without a model store the export stays v1 and carries the bytes", async () => {
		const weights = new TextEncoder().encode("weights");
		const asked = serve(weights);
		const { app, backend } = backendWith(hubModel({ size: weights.length }));
		const exported = await prepareOnlineDependencies(app, backend, profile);
		expect(asked).toEqual(["https://cdn.flow-like.com/bits/qwen3-8b-q4"]);
		const metadata = await metadataOf(exported, "qwen3-8b");
		expect(metadata.version).toBeUndefined();
		expect(metadata.assets).toBeUndefined();
		expect(metadata.artifacts).toHaveLength(1);
		expect(exported.artifact.models).toBeUndefined();
	});
});
