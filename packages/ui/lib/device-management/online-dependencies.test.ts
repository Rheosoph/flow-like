import { afterEach, expect, test } from "bun:test";
import { blake3 } from "@noble/hashes/blake3";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { IApp } from "../schema/app/app";
import { prepareProjectArtifact } from "./artifacts";
import {
	prepareOnlineDependencies,
	publicDependencyMetadata,
} from "./online-dependencies";

const profile = { id: "profile" } as IProfile;
const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});
function fixture(wrapDependencies = false) {
	const app = {
		id: "project",
		bits: ["model"],
		packages: { nodes: "1.0.0" },
	} as unknown as IApp;
	const bit = {
		id: "model",
		hub: "models.test",
		hash: "model",
		file_name: null,
		dependencies: [] as string[],
		parameters: { model: "hosted" },
		download_link: "https://cloud.test/model?token=private",
	};
	const dependencies: (typeof bit)[] = [];
	const calls: unknown[] = [];
	const backend = {
		bitState: { getBit: async () => bit },
		apiState: {
			get: async (_: unknown, path: string) =>
				path === `bit/${bit.id}/dependencies`
					? wrapDependencies
						? { bits: dependencies }
						: dependencies
					: { version: 1, project_id: app.id, documents: { app } },
			post: async (_: unknown, path: string, body: unknown) => {
				calls.push([path, body]);
				return {
					package_id: "nodes",
					version: "1.0.0",
					manifest: { id: "nodes", version: "1.0.0", permissions: {} },
					wasm_base64: btoa("\0asm\x01\0\0\0"),
				};
			},
		},
	} as unknown as IBackendState;
	return { app, backend, bit, dependencies, calls };
}
test("online export resolves and pins dependencies without exporting offline data or URLs", async () => {
	const f = fixture();
	const exported = await prepareOnlineDependencies(f.app, f.backend, profile);
	expect(f.calls).toEqual([
		["registry/download", { package_id: "nodes", version: "1.0.0" }],
	]);
	expect(exported.artifact.descriptor.source).toBe("online");
	expect(exported.assets.bit_pins).toHaveLength(1);
	expect(exported.assets.package_pins).toHaveLength(1);
	expect(
		exported.artifact.files.some((file) => file.path.endsWith("manifest.app")),
	).toBe(false);
	for (const file of exported.artifact.files)
		expect(
			new TextDecoder().decode(await file.file.arrayBuffer()),
		).not.toContain("token=private");
});

test("online export shares one pin for matching plain and qualified workflow references", async () => {
	const f = fixture();
	f.app.bits = ["model", "models.test:model"];
	let reads = 0;
	f.backend.bitState.getBit = async () => {
		reads += 1;
		return {
			...f.bit,
			download_link: `https://cloud.test/model?token=${reads}`,
		} as never;
	};
	const exported = await prepareOnlineDependencies(f.app, f.backend, profile);
	expect(exported.assets.bit_pins).toHaveLength(1);
	expect(exported.artifact.bits).toMatchObject([
		{ id: "model", hub: "models.test" },
	]);
});

test("online export rejects conflicting workflow references with the same Bit ID", async () => {
	const f = fixture();
	f.app.bits = ["model", "other.test:model"];
	f.backend.bitState.getBit = async (_id, hub) =>
		({ ...f.bit, hub: hub ?? f.bit.hub }) as never;
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow('Selected references disagree about model "model".');
});

for (const wrapDependencies of [false, true]) {
	test(`online export resolves qualified transitive dependencies from ${wrapDependencies ? "wrapped" : "array"} inventories`, async () => {
		const f = fixture(wrapDependencies);
		f.app.bits = ["models.test:model"];
		f.bit.dependencies = ["models.test:tokenizer"];
		const tokenizer = {
			...f.bit,
			id: "tokenizer",
			dependencies: ["assets.test:config"],
		};
		const config = {
			...f.bit,
			id: "config",
			hub: "assets.test",
			dependencies: [],
		};
		f.dependencies.push(
			{ ...f.bit, parameters: {} as typeof f.bit.parameters },
			tokenizer,
			config,
		);
		const exported = await prepareOnlineDependencies(f.app, f.backend, profile);
		const metadata = exported.artifact.files.find(
			(file) => file.path === "bits/metadata/model.json",
		);
		if (!metadata) throw new Error("Missing packaged model metadata");
		const text = new TextDecoder().decode(await metadata.file.arrayBuffer());
		expect(JSON.parse(text)).toMatchObject({
			bit: {
				id: "model",
				hub: "models.test",
				dependencies: ["tokenizer"],
				parameters: { model: "hosted" },
			},
			dependencies: [
				{ id: "tokenizer", hub: "models.test", dependencies: ["config"] },
				{ id: "config", hub: "assets.test", dependencies: [] },
			],
		});
		expect(f.bit.dependencies).toEqual(["models.test:tokenizer"]);
		expect(tokenizer.dependencies).toEqual(["assets.test:config"]);
	});
}

test("online export rejects dependencies with the same ID on different hubs", async () => {
	const f = fixture();
	f.bit.dependencies = ["assets.test:model", "other.test:model"];
	f.dependencies.push(
		f.bit,
		{ ...f.bit, hub: "assets.test", dependencies: [] },
		{ ...f.bit, hub: "other.test", dependencies: [] },
	);
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow('Model ID "model" appears on multiple hubs');
	expect(f.calls).toHaveLength(0);
});

test("online export resolves unqualified dependencies by their unique ID", async () => {
	const f = fixture();
	f.bit.dependencies = ["tokenizer"];
	f.dependencies.push({
		...f.bit,
		id: "tokenizer",
		hub: "assets.test",
		dependencies: [],
	});
	const exported = await prepareOnlineDependencies(f.app, f.backend, profile);
	expect(exported.assets.bit_pins).toHaveLength(1);
});

for (const dependency of ["assets.test:missing", "missing"]) {
	test(`online export identifies a missing dependency ${dependency}`, async () => {
		const f = fixture();
		f.bit.dependencies = ["models.test:tokenizer"];
		f.dependencies.push({
			...f.bit,
			id: "tokenizer",
			dependencies: [dependency],
		});
		await expect(
			prepareOnlineDependencies(f.app, f.backend, profile),
		).rejects.toThrow(
			`Model "models.test:tokenizer" is missing dependency "${dependency}" from its resolved inventory.`,
		);
		expect(f.calls).toHaveLength(0);
	});
}

test("online export rejects a dependency returned from the wrong hub", async () => {
	const f = fixture();
	f.bit.dependencies = ["assets.test:tokenizer"];
	f.dependencies.push({
		...f.bit,
		id: "tokenizer",
		hub: "other.test",
		dependencies: [],
	});
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow(
		'Model "models.test:model" is missing dependency "assets.test:tokenizer"',
	);
	expect(f.calls).toHaveLength(0);
});

test("online export rejects a root model returned from the wrong hub", async () => {
	const f = fixture();
	f.app.bits = ["other.test:model"];
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow("A model identity differs from this project.");
	expect(f.calls).toHaveLength(0);
});

test("online artifacts cannot smuggle project database files and cannot masquerade as offline", async () => {
	const marker = {
		path: "apps/project/online-source.json",
		file: new Blob(["online"]),
	};
	await expect(prepareProjectArtifact("project", [marker])).rejects.toThrow();
	await expect(
		prepareProjectArtifact(
			"project",
			[
				marker,
				{
					path: "apps/project/storage/db/table.lance/file",
					file: new Blob(["private"]),
				},
			],
			undefined,
			undefined,
			"online",
		),
	).rejects.toThrow("cannot contain offline");
});
test("embedded provider credentials fail before upload", async () => {
	const f = fixture();
	f.bit.parameters = {
		api_key: "private",
	} as unknown as typeof f.bit.parameters;
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow("embedded credentials");
	expect(f.calls).toHaveLength(0);
	expect(() =>
		publicDependencyMetadata({ provider: { api_key: "private" } }),
	).toThrow();
	expect(
		publicDependencyMetadata({
			provider: { api_key: null },
			download_link: "https://a.test/signed",
			nested: { image: "https://a.test/a?signature=secret" },
		}) as unknown,
	).toEqual({ provider: { api_key: null }, nested: {} });
});
test("a package response must retain the project's exact pinned identity", async () => {
	const f = fixture();
	f.app.packages = { other: "1.0.0" };
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow("pinned version");
});

test("browser exports packages above 64 MiB and dependency totals above 256 MiB", async () => {
	const f = fixture();
	const size = 257 * 1024 ** 2;
	const wasm = new Blob([]);
	Object.defineProperty(wasm, "size", { value: size });
	wasm.arrayBuffer = async () => {
		throw new Error("Read packages in chunks.");
	};
	wasm.slice = (start = 0, end = size) => {
		expect(end - start).toBeLessThanOrEqual(1024 * 1024);
		return new Blob([start === 0 ? "\0asm" : ""]);
	};
	f.backend.apiState.post = (async () => ({
		package_id: "nodes",
		version: "1.0.0",
		manifest: { id: "nodes", version: "1.0.0" },
		download_url: "https://registry.test/nodes.wasm",
	})) as IBackendState["apiState"]["post"];
	globalThis.fetch = (async () => {
		const response = new Response(new Uint8Array(), {
			headers: { "content-length": String(size) },
		});
		response.blob = async () => wasm;
		return response;
	}) as typeof fetch;
	const exported = await prepareOnlineDependencies(f.app, f.backend, profile);
	expect(exported.artifact.descriptor.total_bytes).toBeGreaterThan(size);
	expect(exported.assets.package_pins).toHaveLength(1);
	expect(
		exported.artifact.files.find((entry) => entry.path.endsWith("module.wasm"))
			?.file,
	).toBe(wasm);
});

test("package manifest digests remain verified across Blob chunks", async () => {
	const f = fixture();
	const bytes = new Uint8Array(1024 ** 2 + 4).fill(17);
	bytes.set([0, 97, 115, 109]);
	let digest = Array.from(blake3(bytes), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("");
	f.backend.apiState.post = (async () => ({
		package_id: "nodes",
		version: "1.0.0",
		manifest: { id: "nodes", version: "1.0.0", wasm_hash: digest },
		download_url: "https://registry.test/nodes.wasm",
	})) as IBackendState["apiState"]["post"];
	globalThis.fetch = (async () => new Response(bytes)) as typeof fetch;
	await prepareOnlineDependencies(f.app, f.backend, profile);
	digest = "0".repeat(64);
	await expect(
		prepareOnlineDependencies(f.app, f.backend, profile),
	).rejects.toThrow("manifest digest");
});

test("browser export directs inline projector and case-insensitive MLX sources to native resolution", async () => {
	for (const parameters of [
		{
			provider: {
				params: {
					projection: { download_link: "https://model.test/projector" },
				},
			},
		},
		{
			provider: {
				params: {
					projection: {
						download_link: "https://model.test/projector?signature=private",
					},
				},
			},
		},
		{
			provider: { provider_name: "MLX" },
			huggingface: { repo_id: "owner/model", revision: "a".repeat(40) },
		},
	]) {
		const f = fixture();
		f.bit.parameters = parameters as unknown as typeof f.bit.parameters;
		await expect(
			prepareOnlineDependencies(f.app, f.backend, profile),
		).rejects.toThrow("desktop app");
		expect(f.calls).toHaveLength(0);
	}
});

test("metadata arrays never retain signed media URLs or embedded URL credentials", () => {
	const metadata = publicDependencyMetadata({
		preview_media: [
			"https://media.test/public.webp",
			"https://media.test/a?signature=private",
			["https://user:private@media.test/a", "https://media.test/a#private", 7],
			{
				urls: ["HTTPS://media.test/a?token=private", "https://media.test/kept"],
			},
		],
	});
	expect(metadata as unknown).toEqual({
		preview_media: [
			"https://media.test/public.webp",
			[7],
			{ urls: ["https://media.test/kept"] },
		],
	});
	expect(JSON.stringify(metadata)).not.toContain("private");
});
