import assert from "node:assert/strict";
import { createHash, webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";
import { gzipSync } from "node:zlib";
import { CHANNEL, MAX_DOCUMENT_BYTES, validateCommand } from "./protocol.mjs";

const session = "document-session";
const message = (type, values = {}) => ({
	channel: CHANNEL,
	session,
	type,
	...values,
});

const host = readFileSync(new URL("./host.js", import.meta.url), "utf8");
const loadRuntime = new Function(
	"fetch",
	"crypto",
	`let manifest; ${host.slice(host.indexOf("async function verify("), host.indexOf("\nfunction send("))}\nreturn loadWasm();`,
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const wasmBytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
const compressed = gzipSync(wasmBytes);
function runtimeManifest() {
	return {
		wasm: {
			compression: "gzip",
			byteLength: wasmBytes.byteLength,
			sha256: hash(wasmBytes),
			parts: [
				{
					file: "wordcraft.wasm.gz.000",
					byteLength: compressed.byteLength,
					sha256: hash(compressed),
				},
			],
		},
	};
}

test("loader accepts the upgraded Word runtime and the 256 MiB boundary", async () => {
	for (const byteLength of [197846560, 256 * 1024 * 1024]) {
		const manifest = runtimeManifest();
		manifest.wasm.byteLength = byteLength;
		const reachedPart = new Error("Manifest accepted; part download reached.");
		const requests = [];
		await assert.rejects(
			loadRuntime(async (url) => {
				requests.push(url);
				if (url === "./runtime.json") return Response.json(manifest);
				throw reachedPart;
			}, webcrypto),
			(error) => error === reachedPart,
		);
		assert.deepEqual(requests, ["./runtime.json", "./wordcraft.wasm.gz.000"]);
	}
});

test("loader rejects runtimes above 256 MiB before downloading parts", async () => {
	const manifest = runtimeManifest();
	manifest.wasm.byteLength = 256 * 1024 * 1024 + 1;
	let requests = 0;
	await assert.rejects(
		loadRuntime(async () => {
			requests += 1;
			return Response.json(manifest);
		}, webcrypto),
		/manifest is invalid/,
	);
	assert.equal(requests, 1);
});

test("loader decompresses verified parts and checks full runtime integrity", async () => {
	const manifest = runtimeManifest();
	const bytes = await loadRuntime(
		async (url) =>
			url === "./runtime.json"
				? Response.json(manifest)
				: new Response(compressed),
		webcrypto,
	);
	assert.deepEqual(new Uint8Array(bytes), wasmBytes);
});

test("loader retains part, decompression, and runtime validation", async () => {
	for (const [mutate, pattern] of [
		[
			(manifest) => {
				manifest.wasm.parts[0].file = "../untrusted.gz";
			},
			/manifest is invalid/,
		],
		[
			(manifest) => {
				manifest.wasm.parts[0].byteLength = 8 * 1024 * 1024 + 1;
			},
			/manifest is invalid/,
		],
		[
			(manifest) => {
				manifest.wasm.parts[0].byteLength += 1;
			},
			/download is incomplete/,
		],
		[
			(manifest) => {
				manifest.wasm.parts[0].sha256 = "0".repeat(64);
			},
			/integrity check/,
		],
		[
			(manifest) => {
				manifest.wasm.byteLength += 1;
			},
			/runtime is incomplete/,
		],
		[
			(manifest) => {
				manifest.wasm.sha256 = "0".repeat(64);
			},
			/integrity check/,
		],
	]) {
		const manifest = runtimeManifest();
		mutate(manifest);
		await assert.rejects(
			loadRuntime(
				async (url) =>
					url === "./runtime.json"
						? Response.json(manifest)
						: new Response(compressed),
				webcrypto,
			),
			pattern,
		);
	}
	const invalidGzip = new Uint8Array([1, 2, 3]);
	const manifest = runtimeManifest();
	manifest.wasm.parts[0].byteLength = invalidGzip.byteLength;
	manifest.wasm.parts[0].sha256 = hash(invalidGzip);
	await assert.rejects(
		loadRuntime(
			async (url) =>
				url === "./runtime.json"
					? Response.json(manifest)
					: new Response(invalidGzip),
			webcrypto,
		),
	);
});

test("creates DOCX without input bytes and distinguishes snapshot downloads", () => {
	assert.deepEqual(
		validateCommand(
			message("create", { name: "Untitled.docx", readOnly: false }),
			session,
		),
		{
			type: "create",
			name: "Untitled.docx",
			readOnly: false,
		},
	);
	assert.equal(
		validateCommand(message("create", { name: "Untitled.docx" }), session)
			.readOnly,
		true,
	);
	assert.throws(
		() => validateCommand(message("create", { name: "Untitled.pdf" }), session),
		/DOCX/,
	);
	assert.deepEqual(
		validateCommand(
			message("export", { requestId: "copy", purpose: "download" }),
			session,
		),
		{
			type: "export",
			requestId: "copy",
			purpose: "download",
		},
	);
});

test("ignores messages for another channel or document session", () => {
	assert.equal(
		validateCommand({ ...message("export"), channel: "other" }, session),
		null,
	);
	assert.equal(
		validateCommand(
			message("export", { session: "old", requestId: "1" }),
			session,
		),
		null,
	);
	assert.equal(validateCommand(null, session), null);
});

test("opens DOCX bytes read-only unless editing is explicit", () => {
	const bytes = new ArrayBuffer(4);
	assert.deepEqual(
		validateCommand(message("open", { name: "REPORT.DOCX", bytes }), session),
		{ type: "open", name: "REPORT.DOCX", bytes, readOnly: true },
	);
	assert.equal(
		validateCommand(
			message("open", { name: "report.docx", bytes, readOnly: false }),
			session,
		).readOnly,
		false,
	);
	assert.throws(
		() =>
			validateCommand(message("open", { name: "report.doc", bytes }), session),
		/Only DOCX/,
	);
	assert.throws(
		() =>
			validateCommand(
				message("open", { name: "report.docx", bytes: new ArrayBuffer(0) }),
				session,
			),
		/50 MiB/,
	);
	assert.throws(
		() =>
			validateCommand(
				message("open", {
					name: "report.docx",
					bytes: new ArrayBuffer(MAX_DOCUMENT_BYTES + 1),
				}),
				session,
			),
		/50 MiB/,
	);
});

test("exports are correlated and save acknowledgements require valid revisions", () => {
	assert.deepEqual(
		validateCommand(
			message("export", { requestId: "save-1", format: "txt" }),
			session,
		),
		{ type: "export", requestId: "save-1" },
	);
	assert.equal(validateCommand(message("export"), session), null);
	assert.deepEqual(
		validateCommand(message("saved", { revision: 5 }), session),
		{ type: "saved", revision: 5 },
	);
	for (const revision of [-1, 2.5, Number.NaN, Number.POSITIVE_INFINITY, "5"])
		assert.equal(
			validateCommand(message("saved", { revision }), session),
			null,
		);
});
