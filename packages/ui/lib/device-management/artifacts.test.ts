import { expect, test } from "bun:test";
import { sha256 } from "@noble/hashes/sha2";
import {
	ARTIFACT_CHUNK_BYTES,
	type ArtifactManagementCall,
	type ArtifactTransferStatus,
	ArtifactUploadError,
	type PreparedProjectArtifact,
	parseProjectArtifactAssets,
	prepareProjectArtifact,
	selectedProjectAssetFiles,
	uploadProjectArtifact,
	validateProjectArtifactPath,
} from "./artifacts";
const hex = (v: Uint8Array) =>
	Array.from(v, (b) => b.toString(16).padStart(2, "0")).join("");
async function prepared() {
	return prepareProjectArtifact("project", [
		{
			path: "apps/project/storage/files/data.bin",
			file: new Blob([new Uint8Array(20_000).fill(17)]),
		},
		{ path: "apps/project/storage/files/empty", file: new Blob([]) },
		{ path: "apps/project/manifest.app", file: new Blob(["project metadata"]) },
	]);
}
function server(artifact: PreparedProjectArtifact) {
	let transfer = "";
	let committed = false;
	let ready = false;
	const buffers = new Map<number | null, Uint8Array>();
	const calls: Record<string, unknown>[] = [];
	const status = (index: number | null): ArtifactTransferStatus => {
		const blob =
			index === null
				? new Blob([artifact.manifest])
				: artifact.files[index]?.file;
		if (!blob) throw new Error("index");
		const offset = buffers.get(index)?.length ?? 0;
		const complete =
			(index === null ? ready : buffers.has(index)) && offset === blob.size;
		return {
			transfer_id: transfer,
			descriptor: artifact.descriptor,
			state: committed ? "committed" : "receiving",
			expires_at: Math.floor(Date.now() / 1000) + 3600,
			manifest_ready: ready,
			file_index: index,
			offset,
			complete,
			project_path: committed
				? `/state/projects/project/revisions/${artifact.descriptor.manifest_sha256}`
				: null,
		};
	};
	const request: ArtifactManagementCall = async (command, operation) => {
		expect(command.type).toBe("artifact");
		const request = command.request as Record<string, unknown>;
		calls.push(request);
		if (request.kind === "begin") {
			expect(transfer).toBe("");
			expect(operation).toBeTruthy();
			transfer = operation ?? "";
			return { state: "accepted", result: status(null) };
		}
		expect(request.transfer_id).toBe(transfer);
		expect(request.project_id).toBe("project");
		const index = (request.file_index ?? null) as number | null;
		if (request.kind === "chunk") {
			const bytes = new Uint8Array(
				Buffer.from(request.data as string, "base64url"),
			);
			expect(bytes.length).toBeLessThanOrEqual(ARTIFACT_CHUNK_BYTES);
			expect(
				new TextEncoder().encode(
					JSON.stringify({
						operation_id: crypto.randomUUID(),
						device_id: "device",
						issued_at: 1,
						expires_at: 60,
						command,
					}),
				).length,
			).toBeLessThan(16_384);
			const old = buffers.get(index) ?? new Uint8Array();
			expect(request.offset).toBe(old.length);
			const value = new Uint8Array(old.length + bytes.length);
			value.set(old);
			value.set(bytes, old.length);
			buffers.set(index, value);
			if (index === null && value.length === artifact.manifest.length) {
				expect(hex(sha256(value))).toBe(artifact.descriptor.manifest_sha256);
				ready = true;
			}
		}
		if (request.kind === "commit") {
			for (let i = 0; i < artifact.files.length; i++)
				expect(status(i).complete).toBe(true);
			committed = true;
		}
		return { state: "completed", result: status(index) };
	};
	return { request, calls, buffers };
}
test("hashes a deterministic project manifest and excludes cross-project/private/path aliases", async () => {
	const first = await prepared();
	const next = await prepared();
	expect(first.descriptor).toEqual(next.descriptor);
	const manifest = JSON.parse(new TextDecoder().decode(first.manifest));
	expect(manifest.files[0].path).toBe("apps/project/manifest.app");
	expect(first.descriptor.total_bytes).toBe(20_016);
	for (const path of [
		"apps/other/file",
		"apps/project/../file",
		"apps/project/.secrets/value",
		"apps/project/CON.txt",
		"apps/project/x\\y",
		"apps/project/file.",
		"apps/project/re\u0301sume",
	]) {
		expect(() => validateProjectArtifactPath("project", path)).toThrow();
	}
	validateProjectArtifactPath(
		"project",
		"apps/project/storage/files/résumé.pdf",
	);
	await expect(
		prepareProjectArtifact("project", [
			{ path: "apps/project/manifest.app", file: new Blob(["x"]) },
			{ path: "apps/project/MANIFEST.app", file: new Blob(["y"]) },
		]),
	).rejects.toThrow("colliding");
});
test("streams bounded chunks and confirms all files before committing", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	const progress: number[] = [];
	const result = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		onProgress: (value) => progress.push(value.uploadedBytes),
	});
	expect(result.state).toBe("committed");
	expect(result.project_path).toEndWith(artifact.descriptor.manifest_sha256);
	expect(fake.calls.at(-1)?.kind).toBe("commit");
	expect(progress.at(-1)).toBe(artifact.descriptor.total_bytes);
	expect(fake.buffers.get(1)?.length).toBe(20_000);
});
test("a lost accepted chunk keeps a resumable identity and never retries automatically", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	let failed = false;
	let transfer = "";
	const request: ArtifactManagementCall = async (command, id) => {
		const response = await fake.request(command, id);
		const value = command.request as Record<string, unknown>;
		if (!failed && value.kind === "chunk" && value.file_index === 1) {
			failed = true;
			throw new Error("connection lost with private diagnostic");
		}
		return response;
	};
	try {
		await uploadProjectArtifact({ prepared: artifact, request });
		throw new Error("expected interruption");
	} catch (error) {
		expect(error).toBeInstanceOf(ArtifactUploadError);
		transfer = (error as ArtifactUploadError).transferId;
		expect((error as Error).message).not.toContain("private diagnostic");
	}
	const accepted = fake.buffers.get(1)?.length;
	expect(accepted).toBe(ARTIFACT_CHUNK_BYTES);
	const result = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		transferId: transfer,
	});
	expect(result.state).toBe("committed");
	const chunks = fake.calls.filter(
		(call) => call.kind === "chunk" && call.file_index === 1,
	);
	expect(chunks.map((call) => call.offset)).toEqual([0, 8192, 16384]);
	expect(fake.calls.filter((call) => call.kind === "begin")).toHaveLength(1);
});
test("rejects a substituted status before sending project contents", async () => {
	const artifact = await prepared();
	let count = 0;
	const request: ArtifactManagementCall = async () => {
		count++;
		return { state: "completed", result: { transfer_id: "other" } };
	};
	await expect(
		uploadProjectArtifact({ prepared: artifact, request }),
	).rejects.toBeInstanceOf(ArtifactUploadError);
	expect(count).toBe(1);
});

test("selected Bit and WASM assets are pinned exactly and unrelated assets stay out", async () => {
	const weight = new Blob(["weights"]);
	const wasm = new Blob(["wasm"]);
	const packageManifest = new Blob(["manifest"]);
	const metadata = {
		bit: {
			id: "model",
			hash: "hash",
			file_name: "vision_encoder/weights.bin",
			size: 7,
		},
		dependencies: [],
		artifacts: [
			{
				path: "bits/hash/vision_encoder/weights.bin",
				size: 7,
				sha256: hex(sha256(new TextEncoder().encode("weights"))),
			},
		],
	};
	const metadataBytes = new TextEncoder().encode(JSON.stringify(metadata));
	const assets = parseProjectArtifactAssets(
		JSON.stringify({
			bit_pins: [
				{ bit_id: "model", metadata_sha256: hex(sha256(metadataBytes)) },
			],
			package_pins: [
				{
					package_id: "package",
					version: "1.0.0",
					wasm_sha256: hex(sha256(new TextEncoder().encode("wasm"))),
					manifest_sha256: hex(sha256(new TextEncoder().encode("manifest"))),
				},
			],
		}),
	);
	const input = [
		{ path: "apps/project/manifest.app", file: new Blob(["project"]) },
		{ path: "bits/metadata/model.json", file: new Blob([metadataBytes]) },
		{ path: "bits/hash/vision_encoder/weights.bin", file: weight },
		{ path: "packages/package/1.0.0/module.wasm", file: wasm },
		{ path: "packages/package/1.0.0/manifest.json", file: packageManifest },
	];
	const result = await prepareProjectArtifact(
		"project",
		input,
		undefined,
		assets,
	);
	const manifest = JSON.parse(new TextDecoder().decode(result.manifest));
	expect(manifest.bit_pins).toEqual(assets.bit_pins);
	expect(manifest.package_pins).toEqual(assets.package_pins);
	await expect(
		prepareProjectArtifact(
			"project",
			[
				...input,
				{ path: "bits/other/private.bin", file: new Blob(["private"]) },
			],
			undefined,
			assets,
		),
	).rejects.toThrow("unselected");
	await expect(
		prepareProjectArtifact(
			"project",
			input.map((i) =>
				i.path === "bits/hash/vision_encoder/weights.bin"
					? { ...i, file: new Blob(["WRONG!!"]) }
					: i,
			),
			undefined,
			assets,
		),
	).rejects.toThrow("digest");
	const files = input
		.slice(1)
		.concat([{ path: "bits/other/private.bin", file: new Blob(["private"]) }])
		.map((i) => {
			const file = new File([i.file], i.path.split("/").at(-1) ?? "file");
			Object.defineProperty(file, "webkitRelativePath", {
				value: `store/${i.path}`,
			});
			return file;
		});
	const selected = await selectedProjectAssetFiles(files, assets);
	expect(selected).toHaveLength(4);
	expect(selected.some((file) => file.path.includes("private"))).toBe(false);
});

test("invalid asset JSON never exposes source content in errors", () => {
	let error: unknown;
	try {
		parseProjectArtifactAssets('{"private":"sensitive input"');
	} catch (value) {
		error = value;
	}
	expect(error).toBeInstanceOf(Error);
	expect((error as Error).message).toBe("Selected asset JSON is invalid.");
});

test("pinned nested Bit metadata cannot authorize traversal or private paths", async () => {
	for (const name of [
		"vision_encoder/../weights.bin",
		"vision_encoder//weights.bin",
		"vision_encoder/.secrets/key",
		"vision_encoder\\weights.bin",
	]) {
		const metadata = new TextEncoder().encode(
			JSON.stringify({
				bit: { id: "model", hash: "hash", file_name: name, size: 7 },
				dependencies: [],
				artifacts: [
					{
						path: `bits/hash/${name}`,
						size: 7,
						sha256: hex(sha256(new TextEncoder().encode("weights"))),
					},
				],
			}),
		);
		const assets = {
			bit_pins: [{ bit_id: "model", metadata_sha256: hex(sha256(metadata)) }],
			package_pins: [],
		};
		await expect(
			prepareProjectArtifact(
				"project",
				[
					{ path: "apps/project/manifest.app", file: new Blob(["project"]) },
					{ path: "bits/metadata/model.json", file: new Blob([metadata]) },
					{ path: `bits/hash/${name}`, file: new Blob(["weights"]) },
				],
				undefined,
				assets,
			),
		).rejects.toThrow();
	}
});
