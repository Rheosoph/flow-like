import { expect, test } from "bun:test";
import { sha256 } from "@noble/hashes/sha2";
import {
	ARTIFACT_CHUNK_BYTES,
	ArtifactAbortError,
	type ArtifactManagementCall,
	type ArtifactTransferStatus,
	ArtifactUploadError,
	type PreparedProjectArtifact,
	abortProjectArtifact,
	abortRefusalSettles,
	forgetArtifactTransfer,
	legacyArtifactTransferDevices,
	parseProjectArtifactAssets,
	pendingArtifactTransfers,
	prepareProjectArtifact,
	readArtifactTransfer,
	rememberArtifactTransfer,
	selectedProjectAssetFiles,
	takeLegacyArtifactTransfers,
	uploadProjectArtifact,
	validateProjectArtifactPath,
} from "./artifacts";
import { NativeArtifactUploadError } from "./native-errors";
import {
	ConnectError,
	ManagementReadError,
	ManagementUnconfirmedError,
} from "./transport";
import { DeviceTunnelError, type TunnelArtifactUpload } from "./tunnel";
import { LiveCallError } from "./workspace/errors";
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
		expect(request.kind).not.toBe("chunk");
		if (request.kind === "commit") {
			for (let i = 0; i < artifact.files.length; i++)
				expect(status(i).complete).toBe(true);
			committed = true;
		}
		return { state: "completed", result: status(index) };
	};
	const uploads: { fileIndex: number | null; offset: number }[] = [];
	const upload = async (input: TunnelArtifactUpload, length?: number) => {
		expect(input.transferId).toBe(transfer);
		expect(input.projectId).toBe("project");
		const index = input.fileIndex;
		const old = buffers.get(index) ?? new Uint8Array();
		expect(input.offset).toBe(old.length);
		uploads.push({ fileIndex: index, offset: input.offset });
		const bytes = new Uint8Array(
			await input.file
				.slice(
					input.offset,
					length === undefined ? input.file.size : input.offset + length,
				)
				.arrayBuffer(),
		);
		const value = new Uint8Array(old.length + bytes.length);
		value.set(old);
		value.set(bytes, old.length);
		buffers.set(index, value);
		if (index === null && value.length === artifact.manifest.length) {
			expect(hex(sha256(value))).toBe(artifact.descriptor.manifest_sha256);
			ready = true;
		}
		return status(index);
	};
	return { request, upload, uploads, calls, buffers };
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
test("streams manifest and files separately from management before committing", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	const progress: number[] = [];
	const result = await uploadProjectArtifact({
		upload: fake.upload,
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
test("a lost stream keeps its checkpoint and resumes only when requested", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	let transfer = "";
	try {
		await uploadProjectArtifact({
			prepared: artifact,
			request: fake.request,
			upload: async (input) => {
				if (input.fileIndex !== 1) return fake.upload(input);
				await fake.upload(input, ARTIFACT_CHUNK_BYTES);
				throw new Error("connection lost with private diagnostic");
			},
		});
		throw new Error("expected interruption");
	} catch (error) {
		expect(error).toBeInstanceOf(ArtifactUploadError);
		transfer = (error as ArtifactUploadError).transferId ?? "";
		expect((error as Error).message).not.toContain("private diagnostic");
	}
	expect(fake.buffers.get(1)?.length).toBe(ARTIFACT_CHUNK_BYTES);
	const result = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		upload: fake.upload,
		transferId: transfer,
	});
	expect(result.state).toBe("committed");
	expect(
		fake.uploads
			.filter((value) => value.fileIndex === 1)
			.map((value) => value.offset),
	).toEqual([0, 8192]);
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
		uploadProjectArtifact({
			upload: async () => {
				throw new Error("unexpected upload");
			},
			prepared: artifact,
			request,
		}),
	).rejects.toBeInstanceOf(ArtifactUploadError);
	expect(count).toBe(1);
});

test("a stream must confirm the exact file and its hash before commit", async () => {
	const artifact = await prepared();
	for (const changed of [
		{ transfer_id: "other" },
		{ file_index: 99 },
		{ complete: false },
		{ offset: 0 },
	]) {
		const fake = server(artifact);
		await expect(
			uploadProjectArtifact({
				prepared: artifact,
				request: fake.request,
				upload: async (input) => ({
					...(await fake.upload(input)),
					...changed,
				}),
			}),
		).rejects.toBeInstanceOf(ArtifactUploadError);
		expect(fake.uploads).toHaveLength(1);
		expect(fake.calls.some((call) => call.kind === "commit")).toBe(false);
	}
});

test("a lost final stream reply resumes without sending a verified file again", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	const error = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		upload: async (input) => {
			const result = await fake.upload(input);
			if (input.fileIndex === 1) throw new Error("lost final reply");
			return result;
		},
	}).catch((error) => error as ArtifactUploadError);
	expect(error).toBeInstanceOf(ArtifactUploadError);
	if (!(error instanceof ArtifactUploadError))
		throw new Error("Expected an interrupted upload");
	const result = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		upload: fake.upload,
		transferId: error.transferId,
	});
	expect(result.state).toBe("committed");
	expect(fake.uploads.filter((value) => value.fileIndex === 1)).toHaveLength(1);
	expect(fake.uploads.filter((value) => value.fileIndex === 2)).toHaveLength(1);
	expect(fake.buffers.get(2)?.length).toBe(0);
});

test("upload failures retain safe request and reconnect diagnostics with a resumable transfer", async () => {
	const diagnostic = {
		transport: "websocket",
		phase: "wait_reply",
		cause: "timeout",
		fallbackReason: "ice_timeout",
	} as const;
	const reconnect = new ConnectError(
		"securing",
		"handshake_failed",
		"private handshake error",
		{
			transport: "websocket",
			fallbackReason: "ice_timeout",
			url: "wss://private.example/ws/devices",
		},
	);
	const failures = [
		[new ManagementUnconfirmedError("op", diagnostic), diagnostic],
		[reconnect, reconnect.diagnostic],
		[
			new LiveCallError(
				"handshake_failed",
				reconnect.message,
				{ step: "securing", code: "handshake_failed" },
				reconnect.diagnostic,
			),
			reconnect.diagnostic,
		],
		[new Error("private local read failure"), undefined],
	] as const;
	for (const [failure, expected] of failures) {
		const artifact = await prepared();
		const fake = server(artifact);
		const error = await uploadProjectArtifact({
			prepared: artifact,
			request: fake.request,
			upload: async (input) => {
				await fake.upload(input);
				throw failure;
			},
		}).catch((error) => error);
		expect(error).toBeInstanceOf(ArtifactUploadError);
		expect(error.diagnostic).toEqual(expected);
		expect(error.rejection).toBeUndefined();
		expect(JSON.stringify(error)).not.toContain("private");
		expect(error.message).not.toContain("private");
		expect(fake.uploads).toHaveLength(1);
		const resumed = await uploadProjectArtifact({
			upload: fake.upload,
			prepared: artifact,
			request: fake.request,
			transferId: error.transferId,
		});
		expect(resumed.state).toBe("committed");
		expect(fake.calls.filter((call) => call.kind === "begin")).toHaveLength(1);
	}
});

test("upload diagnostics retain stream failure codes and positions without peer error text", async () => {
	const artifact = await prepared();
	for (const [code, cause] of [
		["open_timeout", "open_timeout"],
		["connection_closed", "connection_closed"],
		["stream_failed", "stream_failed"],
		["private_peer_code", "tunnel_failed"],
	] as const) {
		const fake = server(artifact);
		const error = await uploadProjectArtifact({
			prepared: artifact,
			request: fake.request,
			upload: async (input) => {
				if (input.fileIndex === 0)
					throw new DeviceTunnelError(code, "private /project/token=secret");
				return fake.upload(input);
			},
		}).catch((error: unknown) => error);
		expect(error).toBeInstanceOf(ArtifactUploadError);
		if (!(error instanceof ArtifactUploadError)) throw error;
		expect(error.uploadDiagnostic).toEqual({
			phase: "file",
			cause,
			fileIndex: 0,
			offset: 0,
		});
		expect(error.transferId).toBeString();
		expect(error.rejection).toBeUndefined();
		expect(JSON.stringify(error)).not.toContain("private");
		expect(error.message).not.toContain("private");
		const resumed = await uploadProjectArtifact({
			prepared: artifact,
			request: fake.request,
			upload: fake.upload,
			transferId: error.transferId,
		});
		expect(resumed.state).toBe("committed");
	}
});

test("tunnel setup read failures retain their safe cause in upload diagnostics", async () => {
	const artifact = await prepared();
	for (const code of [
		"timeout",
		"connection_closed",
		"invalid_reply",
	] as const) {
		const fake = server(artifact);
		const error = await uploadProjectArtifact({
			prepared: artifact,
			request: fake.request,
			upload: async () => {
				throw new ManagementReadError(code, "private transport details");
			},
		}).catch((error: unknown) => error);
		expect(error).toBeInstanceOf(ArtifactUploadError);
		if (!(error instanceof ArtifactUploadError)) throw error;
		expect(error.uploadDiagnostic).toEqual({
			phase: "manifest",
			cause: code,
			fileIndex: null,
			offset: 0,
		});
		expect(JSON.stringify(error)).not.toContain("private");
	}
});

test("native upload diagnostics reach the resumable artifact failure", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	const error = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		upload: async (input) => {
			if (input.fileIndex === 0)
				throw new NativeArtifactUploadError("connect", "connection_failed");
			return fake.upload(input);
		},
	}).catch((error: unknown) => error);
	expect(error).toBeInstanceOf(ArtifactUploadError);
	if (!(error instanceof ArtifactUploadError)) throw error;
	expect(error.uploadDiagnostic).toEqual({
		phase: "file",
		cause: "connection_failed",
		fileIndex: 0,
		offset: 0,
		nativePhase: "connect",
	});
	expect(error.transferId).toBeString();
	expect(error.rejection).toBeUndefined();
});

test("upload diagnostics distinguish management phases and local read failures", async () => {
	const artifact = await prepared();
	for (const phase of ["begin", "resume", "file_status", "commit"] as const) {
		const fake = server(artifact);
		const error = await uploadProjectArtifact({
			prepared: artifact,
			...(phase === "resume" ? { transferId: crypto.randomUUID() } : {}),
			request: async (command, operationId) => {
				const request = command.request as Record<string, unknown>;
				if (
					request.kind === phase ||
					(request.kind === "status" &&
						(phase === "resume" || phase === "file_status"))
				)
					throw new ManagementUnconfirmedError("op", {
						phase: "wait_reply",
						cause: "timeout",
					});
				return fake.request(command, operationId);
			},
			upload: fake.upload,
		}).catch((error: unknown) => error);
		expect(error).toBeInstanceOf(ArtifactUploadError);
		if (!(error instanceof ArtifactUploadError)) throw error;
		expect(error.uploadDiagnostic).toEqual({
			phase,
			cause: "management_failed",
			...(phase === "file_status" ? { fileIndex: 0 } : {}),
		});
	}
	const fake = server(artifact);
	const local = await uploadProjectArtifact({
		prepared: artifact,
		request: fake.request,
		upload: async () => {
			throw new Error("private file could not be opened");
		},
	}).catch((error: unknown) => error);
	expect(local).toBeInstanceOf(ArtifactUploadError);
	if (!(local instanceof ArtifactUploadError)) throw local;
	expect(local.uploadDiagnostic).toEqual({
		phase: "manifest",
		cause: "upload_failed",
		fileIndex: null,
		offset: 0,
	});
	expect(JSON.stringify(local)).not.toContain("private");
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

test("asset pins are rebuilt in the device's canonical field order", () => {
	const digest = "a".repeat(64);
	const selection = parseProjectArtifactAssets(
		JSON.stringify({
			package_pins: [
				{
					manifest_sha256: digest,
					wasm_sha256: digest,
					version: "1.0.0",
					package_id: "package",
				},
			],
			bit_pins: [{ metadata_sha256: digest, bit_id: "model" }],
		}),
	);
	expect(JSON.stringify(selection)).toBe(
		JSON.stringify({
			bit_pins: [{ bit_id: "model", metadata_sha256: digest }],
			package_pins: [
				{
					package_id: "package",
					version: "1.0.0",
					wasm_sha256: digest,
					manifest_sha256: digest,
				},
			],
		}),
	);
	const packages = Array.from({ length: 65 }, (_, index) => ({
		package_id: `package-${index}`,
		version: "1.0.0",
		wasm_sha256: digest,
		manifest_sha256: digest,
	}));
	expect(() =>
		parseProjectArtifactAssets(JSON.stringify({ package_pins: packages })),
	).toThrow("at most 256 Bits and 64 WASM packages");
});

test("asset pins with several versions of one node package fail before any upload", () => {
	const digest = "a".repeat(64);
	const pin = (version: string) => ({
		package_id: "tokenizer",
		version,
		wasm_sha256: digest,
		manifest_sha256: digest,
	});
	expect(
		parseProjectArtifactAssets(JSON.stringify({ package_pins: [pin("1.0.0")] }))
			.package_pins,
	).toEqual([pin("1.0.0")]);
	expect(() =>
		parseProjectArtifactAssets(
			JSON.stringify({ package_pins: [pin("1.0.0"), pin("2.0.0")] }),
		),
	).toThrow("several versions of node package tokenizer");
});

function rejection(code: string, error = `${code} cause`) {
	return {
		state: "rejected",
		result: { error, code, retryable: ["busy", "failed"].includes(code) },
	};
}

test("busy artifact metadata requests back off while the device lock is held", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	let busy = 2;
	const result = await uploadProjectArtifact({
		upload: fake.upload,
		prepared: artifact,
		request: async (command, id) => {
			const request = command.request as Record<string, unknown>;
			if (request.kind === "begin" && busy > 0) {
				busy--;
				return rejection("busy");
			}
			return fake.request(command, id);
		},
	});
	expect(result.state).toBe("committed");
	expect(busy).toBe(0);
});

test("definitive artifact rejections carry their reason and whether a transfer exists", async () => {
	const artifact = await prepared();
	const begin = (await uploadProjectArtifact({
		upload: async () => {
			throw new Error("unexpected upload");
		},
		prepared: artifact,
		request: async () => rejection("limit", "Artifact staging quota exceeded"),
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(begin).toBeInstanceOf(ArtifactUploadError);
	expect(begin.transferId).toBeUndefined();
	expect(begin.message).toContain("Artifact staging quota exceeded");
	expect(begin.message).toContain("Abort unfinished uploads");
	const fake = server(artifact);
	const binding = (await uploadProjectArtifact({
		upload: fake.upload,
		prepared: artifact,
		request: async (command, id) =>
			(command.request as Record<string, unknown>).kind === "commit"
				? rejection("invalid", "Artifact manifest binding differs")
				: fake.request(command, id),
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(binding.transferId).toBeString();
	expect(binding.message).toContain("Artifact manifest binding differs");
	expect(binding.message).toContain("Abort this transfer");
	const transferId = crypto.randomUUID();
	const refused = (await abortProjectArtifact(
		async () => rejection("unauthorized", "Only its uploader can abort"),
		"project",
		transferId,
	).catch((error: unknown) => error)) as ArtifactAbortError;
	expect(refused).toBeInstanceOf(ArtifactAbortError);
	expect(refused.rejection?.code).toBe("unauthorized");
	expect(refused.message).toContain("Only its uploader can abort");
});

test("retryable artifact rejections keep the transfer resumable and show the device's reason", async () => {
	const artifact = await prepared();
	const failed = (await uploadProjectArtifact({
		upload: async () => {
			throw new Error("unexpected upload");
		},
		prepared: artifact,
		request: async () => rejection("failed", "Artifact staging disk is full"),
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(failed).toBeInstanceOf(ArtifactUploadError);
	expect(failed.transferId).toBeString();
	expect(failed.rejection?.retryable).toBe(true);
	expect(failed.message).toContain("Artifact staging disk is full");
	expect(failed.message).toContain("Resume this transfer");
	const legacy = (await uploadProjectArtifact({
		upload: async () => {
			throw new Error("unexpected upload");
		},
		prepared: artifact,
		request: async () => ({
			state: "rejected",
			result: { error: "Command rejected" },
		}),
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(legacy.transferId).toBeString();
	expect(legacy.rejection).toBeUndefined();
	expect(legacy.message).toContain("no confirmed completion");
	const transferId = crypto.randomUUID();
	const unconfirmed = (await abortProjectArtifact(
		async () => rejection("failed", "Staging database is locked"),
		"project",
		transferId,
	).catch((error: unknown) => error)) as ArtifactAbortError;
	expect(unconfirmed).toBeInstanceOf(ArtifactAbortError);
	expect(unconfirmed.rejection).toBeUndefined();
	expect(unconfirmed.message).toContain("Staging database is locked");
	expect(unconfirmed.message).toContain("Reconnect and try again");
});

test("resuming a transfer whose begin never reached the device begins it under the same id", async () => {
	const artifact = await prepared();
	const fake = server(artifact);
	const transferId = crypto.randomUUID();
	const request: ArtifactManagementCall = async (command, id) =>
		(command.request as Record<string, unknown>).kind === "status" &&
		!fake.calls.some((call) => call.kind === "begin")
			? rejection("failed", "Unknown artifact transfer")
			: fake.request(command, id);
	const confirmed = (await uploadProjectArtifact({
		upload: fake.upload,
		prepared: artifact,
		request,
		transferId,
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(confirmed).toBeInstanceOf(ArtifactUploadError);
	expect(confirmed.transferId).toBe(transferId);
	expect(fake.calls).toHaveLength(0);
	const result = await uploadProjectArtifact({
		upload: fake.upload,
		prepared: artifact,
		request,
		transferId,
		confirmed: false,
	});
	expect(result.state).toBe("committed");
	expect(result.transfer_id).toBe(transferId);
	expect(fake.calls.filter((call) => call.kind === "begin")).toHaveLength(1);
});

test("an unacknowledged transfer the device refuses to begin again is no longer resumable", async () => {
	const artifact = await prepared();
	const transferId = crypto.randomUUID();
	const kinds: unknown[] = [];
	const journaled = (await uploadProjectArtifact({
		upload: async () => {
			throw new Error("unexpected upload");
		},
		prepared: artifact,
		request: async (command) => {
			const { kind } = command.request as Record<string, unknown>;
			kinds.push(kind);
			return kind === "status"
				? rejection("failed", "Unknown artifact transfer")
				: rejection(
						"invalid",
						`Operation ID ${transferId} already belongs to a different request`,
					);
		},
		transferId,
		confirmed: false,
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(kinds).toEqual(["status", "begin"]);
	expect(journaled.transferId).toBeUndefined();
	expect(journaled.message).toContain("already belongs to a different request");
	kinds.length = 0;
	const denied = (await uploadProjectArtifact({
		upload: async () => {
			throw new Error("unexpected upload");
		},
		prepared: artifact,
		request: async (command) => {
			kinds.push((command.request as Record<string, unknown>).kind);
			return rejection("unauthorized", "Deploy access expired");
		},
		transferId,
		confirmed: false,
	}).catch((error: unknown) => error)) as ArtifactUploadError;
	expect(kinds).toEqual(["status"]);
	expect(denied.transferId).toBeUndefined();
});

test("refused aborts settle unknown transfers but keep busy ones and acknowledged ones an older agent refuses", async () => {
	const refuse = (response: { state: string; result: unknown }) =>
		abortProjectArtifact(
			async () => response,
			"project",
			crypto.randomUUID(),
		).catch((error: unknown) => error);
	const missing = await refuse(
		rejection("failed", "Unknown artifact transfer"),
	);
	expect(abortRefusalSettles(missing, true)).toBe(true);
	expect(abortRefusalSettles(missing, false)).toBe(true);
	expect(
		abortRefusalSettles(await refuse(rejection("unauthorized")), true),
	).toBe(true);
	const legacy = await refuse({
		state: "rejected",
		result: { error: "Command rejected" },
	});
	expect(abortRefusalSettles(legacy, false)).toBe(true);
	expect(abortRefusalSettles(legacy, true)).toBe(false);
	const busy = new ArtifactAbortError(crypto.randomUUID(), {
		code: "busy",
		error: "Artifact lock is held",
		retryable: true,
	});
	expect(abortRefusalSettles(busy, false)).toBe(false);
	expect(
		abortRefusalSettles(
			await refuse({ state: "completed", result: { state: "receiving" } }),
			false,
		),
	).toBe(false);
	expect(abortRefusalSettles(new Error("session closed"), false)).toBe(false);
});

test("unfinished transfers persist per device until committed, aborted or expired", () => {
	const values = new Map<string, string>();
	const previous = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => values.set(key, value),
			removeItem: (key: string) => values.delete(key),
		},
	});
	try {
		const transfer = {
			transfer_id: crypto.randomUUID(),
			project_id: "project",
			manifest_sha256: "a".repeat(64),
		};
		rememberArtifactTransfer("device", transfer);
		rememberArtifactTransfer("device", transfer);
		expect(pendingArtifactTransfers("device", "project")).toHaveLength(1);
		expect(pendingArtifactTransfers("device", "other")).toHaveLength(0);
		expect(pendingArtifactTransfers("another-device")).toHaveLength(0);
		const [first] = pendingArtifactTransfers("device");
		expect(first?.confirmed).toBeUndefined();
		rememberArtifactTransfer("device", { ...transfer, confirmed: true });
		rememberArtifactTransfer("device", { ...transfer, confirmed: false });
		expect(pendingArtifactTransfers("device")).toEqual([
			{ ...transfer, confirmed: true, expires_at: first?.expires_at ?? 0 },
		]);
		forgetArtifactTransfer("device", transfer.transfer_id);
		expect(pendingArtifactTransfers("device")).toHaveLength(0);
		values.set(
			"flow-like.device-artifact-transfers.device",
			JSON.stringify([{ ...transfer, expires_at: 1 }, { broken: true }]),
		);
		expect(pendingArtifactTransfers("device")).toHaveLength(0);
		values.set("flow-like.device-artifact-transfers.device", "{not json");
		expect(pendingArtifactTransfers("device")).toEqual([]);
	} finally {
		if (previous) Object.defineProperty(globalThis, "localStorage", previous);
		else Reflect.deleteProperty(globalThis, "localStorage");
	}
});

function withMemoryStorage(run: (values: Map<string, string>) => void) {
	const values = new Map<string, string>();
	const previous = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			get length() {
				return values.size;
			},
			key: (index: number) => [...values.keys()][index] ?? null,
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => values.set(key, value),
			removeItem: (key: string) => values.delete(key),
		},
	});
	try {
		run(values);
	} finally {
		if (previous) Object.defineProperty(globalThis, "localStorage", previous);
		else Reflect.deleteProperty(globalThis, "localStorage");
	}
}

const account = (name: string, apiOrigin = "https://hub.example") => ({
	issuer: "https://id.example",
	account: name,
	apiOrigin,
	profileId: "default",
});

test("scoped upload hints belong to one account and hub", () => {
	withMemoryStorage(() => {
		const transfer = {
			transfer_id: crypto.randomUUID(),
			project_id: "project",
			manifest_sha256: "a".repeat(64),
		};
		rememberArtifactTransfer("device", transfer, account("ana"));
		expect(
			pendingArtifactTransfers("device", "project", account("ana")),
		).toHaveLength(1);
		expect(
			pendingArtifactTransfers("device", undefined, account("ben")),
		).toEqual([]);
		expect(
			pendingArtifactTransfers(
				"device",
				undefined,
				account("ana", "https://other.example"),
			),
		).toEqual([]);
		expect(pendingArtifactTransfers("device")).toEqual([]);
		expect(legacyArtifactTransferDevices()).toEqual([]);
		forgetArtifactTransfer("device", transfer.transfer_id, account("ana"));
		expect(
			pendingArtifactTransfers("device", undefined, account("ana")),
		).toEqual([]);
	});
});

test("legacy upload hints are listed and taken exactly once", () => {
	withMemoryStorage((values) => {
		const transfer = {
			transfer_id: crypto.randomUUID(),
			project_id: "project",
			manifest_sha256: "a".repeat(64),
			confirmed: true,
		};
		rememberArtifactTransfer("edge-1", transfer);
		values.set(
			"flow-like.device-artifact-transfers.edge-2",
			JSON.stringify([{ ...transfer, expires_at: 1 }]),
		);
		values.set("unrelated", "1");
		expect(legacyArtifactTransferDevices().sort()).toEqual([
			"edge-1",
			"edge-2",
		]);
		expect(takeLegacyArtifactTransfers("edge-1")).toEqual([
			expect.objectContaining(transfer),
		]);
		expect(takeLegacyArtifactTransfers("edge-1")).toEqual([]);
		expect(takeLegacyArtifactTransfers("edge-2")).toEqual([]);
		expect(legacyArtifactTransferDevices()).toEqual([]);
		expect(values.get("unrelated")).toBe("1");
	});
});

test("an upload's device status is read for resuming, and a forgotten transfer reads as gone", async () => {
	const artifact = await prepared();
	const transfer = {
		transfer_id: crypto.randomUUID(),
		project_id: "project",
		manifest_sha256: artifact.descriptor.manifest_sha256,
	};
	const status: ArtifactTransferStatus = {
		transfer_id: transfer.transfer_id,
		descriptor: artifact.descriptor,
		state: "receiving",
		expires_at: 2_000_000_000,
		manifest_ready: true,
		file_index: null,
		offset: artifact.descriptor.manifest_size,
		complete: true,
		project_path: null,
	};
	const sent: Record<string, unknown>[] = [];
	const reply =
		(response: { state: string; result: unknown }): ArtifactManagementCall =>
		async (command) => {
			sent.push(command);
			return response;
		};
	expect(
		await readArtifactTransfer(
			reply({ state: "completed", result: status }),
			transfer,
		),
	).toEqual(status);
	expect(sent[0]).toEqual({
		type: "artifact",
		request: {
			kind: "status",
			project_id: "project",
			transfer_id: transfer.transfer_id,
			file_index: null,
		},
	});
	expect(
		await readArtifactTransfer(
			reply({
				state: "rejected",
				result: { code: "failed", error: "Unknown transfer", retryable: true },
			}),
			transfer,
		),
	).toBeNull();
	await expect(
		readArtifactTransfer(
			reply({
				state: "completed",
				result: {
					...status,
					descriptor: { ...status.descriptor, manifest_sha256: "f".repeat(64) },
				},
			}),
			transfer,
		),
	).rejects.toThrow("does not match this upload");
	await expect(
		readArtifactTransfer(
			reply({
				state: "rejected",
				result: { code: "unauthorized", error: "No Deploy access." },
			}),
			transfer,
		),
	).rejects.toThrow("No Deploy access.");
});
