import { expect, test } from "bun:test";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { IApp } from "../schema/app/app";
import { prepareProjectArtifact } from "./artifacts";
import {
	prepareOnlineDependencies,
	publicDependencyMetadata,
} from "./online-dependencies";

const profile = { id: "profile" } as IProfile;
function fixture() {
	const app = {
		id: "project",
		bits: ["model"],
		packages: { nodes: "1.0.0" },
	} as unknown as IApp;
	const bit = {
		id: "model",
		hash: "model",
		file_name: null,
		dependencies: [],
		parameters: { model: "hosted" },
		download_link: "https://cloud.test/model?token=private",
	};
	const calls: unknown[] = [];
	const backend = {
		bitState: { getBit: async () => bit },
		apiState: {
			get: async () => ({ version: 1, project_id: app.id, documents: { app } }),
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
	return { app, backend, bit, calls };
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
test("unresolved dependency closure and embedded provider credentials fail before upload", async () => {
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
