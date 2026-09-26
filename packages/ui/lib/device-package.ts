export type ReleaseTarget =
	| "x86_64-unknown-linux-gnu"
	| "aarch64-unknown-linux-gnu"
	| "x86_64-apple-darwin"
	| "aarch64-apple-darwin";
export interface ReleaseConfig {
	manifestUrl: string;
	publicKeys: string[];
	minimumSequence: number;
}
export interface StandaloneArtifact {
	target: ReleaseTarget;
	url: string;
	size: number;
	sha256: string;
}
export interface StandaloneRelease {
	version: 1;
	state_schema_version: number;
	sequence: number;
	release_version: string;
	issued_at: number;
	expires_at: number;
	artifacts: StandaloneArtifact[];
	container: { image: string; platforms: string[] } | null;
}
export interface VerifiedRelease {
	manifest: StandaloneRelease;
	manifestJws: string;
	signerFingerprint: string;
	targets: ReleaseTarget[];
}
export interface StandalonePackageInput {
	manifest: unknown;
	manifest_jws: string;
	enrollment_token: string;
	bootstrap_secret: number[];
	target: ReleaseTarget;
	mode: "binary" | "docker" | "both";
	release: ReleaseConfig;
	verifiedRelease?: VerifiedRelease;
	signal?: AbortSignal;
}

const TARGETS: readonly ReleaseTarget[] = [
	"x86_64-unknown-linux-gnu",
	"aarch64-unknown-linux-gnu",
	"x86_64-apple-darwin",
	"aarch64-apple-darwin",
];
const MAX_JWS = 16 * 1024;
const MAX_BROWSER_BINARY = 256 * 1024 * 1024;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const fail = (message: string): never => {
	throw new Error(message);
};
const assert: (condition: unknown, message: string) => asserts condition = (
	condition,
	message,
) => {
	if (!condition) fail(message);
};
function record(value: unknown): Record<string, unknown> {
	assert(
		value !== null && typeof value === "object" && !Array.isArray(value),
		"Invalid package metadata.",
	);
	return value as Record<string, unknown>;
}
function exact(value: Record<string, unknown>, fields: string[]) {
	assert(
		Object.keys(value).length === fields.length &&
			fields.every((field) => Object.hasOwn(value, field)),
		"Unexpected package metadata fields.",
	);
}
function encode(bytes: Uint8Array): string {
	let value = "";
	for (let index = 0; index < bytes.length; index += 8192)
		value += String.fromCharCode(...bytes.subarray(index, index + 8192));
	return btoa(value)
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replace(/=+$/, "");
}
function decode(value: string): Uint8Array<ArrayBuffer> {
	assert(/^[A-Za-z0-9_-]+$/.test(value), "Invalid package signature encoding.");
	const bytes = Uint8Array.from(
		atob(value.replaceAll("-", "+").replaceAll("_", "/")),
		(character) => character.charCodeAt(0),
	);
	assert(encode(bytes) === value, "Noncanonical package signature encoding.");
	return bytes;
}
function publicKey(value: string): Uint8Array<ArrayBuffer> {
	const bytes = decode(value);
	assert(bytes.length === 32, "Invalid Ed25519 public key size.");
	const y = bytes.slice();
	y[31] = (y[31] ?? 0) & 0x7f;
	const hex = [...y].map((byte) => byte.toString(16).padStart(2, "0")).join("");
	const integer = BigInt(
		`0x${[...y]
			.reverse()
			.map((byte) => byte.toString(16).padStart(2, "0"))
			.join("")}`,
	);
	assert(integer < (1n << 255n) - 19n, "Noncanonical Ed25519 public key.");
	// The eight small-order Edwards points have these five y coordinates.
	assert(
		![
			"0000000000000000000000000000000000000000000000000000000000000000",
			"0100000000000000000000000000000000000000000000000000000000000000",
			"ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
			"c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
			"26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
		].includes(hex),
		"Small-order Ed25519 public keys are not accepted.",
	);
	return bytes;
}
function canonical(value: unknown): string {
	if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
	if (value !== null && typeof value === "object") {
		const parts = Object.entries(value)
			.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
			.map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`)
			.join(",");
		return `{${parts}}`;
	}
	const encoded = JSON.stringify(value);
	assert(encoded !== undefined, "Package metadata must contain JSON values.");
	return encoded;
}

async function sha256(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
	return [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))]
		.map((byte) => byte.toString(16).padStart(2, "0"))
		.join("");
}
function https(value: string): void {
	const url = new URL(value);
	assert(
		value.length <= 1024 &&
			url.protocol === "https:" &&
			!url.username &&
			!url.password &&
			!url.search &&
			!url.hash &&
			url.href === value,
		"Release downloads require a canonical HTTPS URL without credentials, query, or fragment.",
	);
}

/** Reject duplicate keys before JSON.parse discards the earlier value. */
function strictJson(text: string): unknown {
	let index = 0;
	const space = () => {
		while (/[\t\r\n ]/.test(text[index] ?? "")) index++;
	};
	const string = (): string => {
		const start = index++;
		while (index < text.length) {
			if (text[index] === "\\") index += 2;
			else if (text[index++] === '"')
				return JSON.parse(text.slice(start, index));
		}
		return fail("Invalid package JSON string.");
	};
	const value = (depth: number): void => {
		assert(depth <= 16, "Package JSON nesting exceeds its limit.");
		space();
		if (text[index] === '"') {
			string();
			return;
		}
		if (text[index] === "{" || text[index] === "[") {
			const object = text[index++] === "{";
			const end = object ? "}" : "]";
			const keys = new Set<string>();
			space();
			if (text[index] === end) {
				index++;
				return;
			}
			while (index < text.length) {
				space();
				if (object) {
					assert(text[index] === '"', "Invalid package JSON key.");
					const key = string();
					assert(!keys.has(key), "Duplicate package metadata field.");
					keys.add(key);
					space();
					assert(text[index++] === ":", "Invalid package JSON object.");
				}
				value(depth + 1);
				space();
				if (text[index] === end) {
					index++;
					return;
				}
				assert(text[index++] === ",", "Invalid package JSON separator.");
			}
			fail("Incomplete package JSON.");
		}
		const start = index;
		while (index < text.length && !/[\s,\]}]/.test(text[index] ?? "")) index++;
		assert(index > start, "Invalid package JSON value.");
		JSON.parse(text.slice(start, index));
	};
	value(0);
	space();
	assert(index === text.length, "Unexpected package JSON content.");
	return JSON.parse(text);
}

export function validateReleaseConfig(input: ReleaseConfig): ReleaseConfig {
	https(input.manifestUrl);
	assert(
		Array.isArray(input.publicKeys) &&
			input.publicKeys.length > 0 &&
			input.publicKeys.length <= 8,
		"Configure trusted standalone release public keys before creating a package.",
	);
	for (const key of input.publicKeys)
		assert(
			typeof key === "string" && publicKey(key).length === 32,
			"A release public key must contain 32 canonical base64url bytes.",
		);
	assert(
		new Set(input.publicKeys).size === input.publicKeys.length,
		"Duplicate release public key.",
	);
	assert(
		Number.isSafeInteger(input.minimumSequence) && input.minimumSequence >= 0,
		"Invalid minimum release sequence.",
	);
	return {
		manifestUrl: input.manifestUrl,
		publicKeys: [...input.publicKeys],
		minimumSequence: input.minimumSequence,
	};
}

async function verifyJws(
	compact: string,
	publicKeys: string[],
	type: string,
): Promise<{ payload: unknown; fingerprint: string }> {
	assert(compact.length <= MAX_JWS, "Package signature exceeds its limit.");
	const parts = compact.split(".");
	assert(parts.length === 3, "Invalid package signature.");
	const [encodedHeader, encodedPayload, encodedSignature] = parts;
	assert(
		encodedHeader && encodedPayload && encodedSignature,
		"Incomplete package signature.",
	);
	const header = record(strictJson(decoder.decode(decode(encodedHeader))));
	exact(header, ["alg", "typ", "kid"]);
	assert(
		header.alg === "EdDSA" &&
			header.typ === type &&
			typeof header.kid === "string",
		"Package signature profile mismatch.",
	);
	const signature = decode(encodedSignature);
	assert(signature.length === 64, "Invalid Ed25519 signature size.");
	for (const raw of publicKeys) {
		const bytes = publicKey(raw);
		assert(bytes.length === 32, "Invalid release public key.");
		const canonical = encoder.encode(
			JSON.stringify({ crv: "Ed25519", kty: "OKP", x: raw }),
		);
		const fingerprint = encode(
			new Uint8Array(await crypto.subtle.digest("SHA-256", canonical)),
		);
		if (fingerprint !== header.kid) continue;
		const key = await crypto.subtle.importKey("raw", bytes, "Ed25519", false, [
			"verify",
		]);
		assert(
			await crypto.subtle.verify(
				"Ed25519",
				key,
				signature,
				encoder.encode(`${encodedHeader}.${encodedPayload}`),
			),
			"Release signature verification failed.",
		);
		return {
			payload: strictJson(decoder.decode(decode(encodedPayload))),
			fingerprint,
		};
	}
	return fail("The package signature does not match a configured release key.");
}

function validateManifest(
	input: unknown,
	config: ReleaseConfig,
	now: number,
): StandaloneRelease {
	const value = record(input);
	exact(value, [
		"version",
		"state_schema_version",
		"sequence",
		"release_version",
		"issued_at",
		"expires_at",
		"artifacts",
		"container",
	]);
	assert(
		value.version === 1 &&
			Number.isSafeInteger(value.state_schema_version) &&
			Number(value.state_schema_version) > 0 &&
			Number(value.state_schema_version) <= 0xffffffff &&
			Number.isSafeInteger(value.sequence) &&
			Number(value.sequence) > 0 &&
			Number(value.sequence) >= config.minimumSequence,
		"Release version or sequence is invalid or older than the configured minimum.",
	);
	assert(
		typeof value.release_version === "string" &&
			value.release_version.length <= 64 &&
			/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][A-Za-z\d-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][A-Za-z\d-]*))*)?(?:\+[A-Za-z\d-]+(?:\.[A-Za-z\d-]+)*)?$/.test(
				value.release_version,
			),
		"Invalid standalone release version.",
	);
	assert(
		value.release_version
			.split(/[-+]/)[0]
			?.split(".")
			.every((part) => BigInt(part) <= 18_446_744_073_709_551_615n),
		"Release version component exceeds its limit.",
	);
	assert(
		Number.isSafeInteger(value.issued_at) &&
			Number.isSafeInteger(value.expires_at) &&
			Number(value.issued_at) >= 0 &&
			Number(value.issued_at) <= now &&
			Number(value.expires_at) > now &&
			Number(value.expires_at) - Number(value.issued_at) <= 30 * 86400,
		"The release manifest has expired or has an invalid lifetime.",
	);
	assert(
		Array.isArray(value.artifacts) &&
			value.artifacts.length > 0 &&
			value.artifacts.length <= 4,
		"Invalid release artifact count.",
	);
	const targets = new Set<unknown>();
	for (const item of value.artifacts) {
		const artifact = record(item);
		exact(artifact, ["target", "url", "size", "sha256"]);
		assert(
			TARGETS.includes(artifact.target as ReleaseTarget) &&
				!targets.has(artifact.target),
			"Unsupported or duplicate release target.",
		);
		targets.add(artifact.target);
		assert(typeof artifact.url === "string", "Invalid release artifact URL.");
		https(artifact.url);
		assert(
			Number.isSafeInteger(artifact.size) &&
				Number(artifact.size) > 0 &&
				Number(artifact.size) <= 2 * 1024 ** 3 &&
				typeof artifact.sha256 === "string" &&
				/^[a-f0-9]{64}$/.test(artifact.sha256),
			"Invalid release artifact digest or size.",
		);
	}
	if (value.container !== null) {
		const container = record(value.container);
		exact(container, ["image", "platforms"]);
		assert(
			typeof container.image === "string" &&
				/^[a-z0-9][a-z0-9/._:-]{1,254}@sha256:[a-f0-9]{64}$/.test(
					container.image,
				) &&
				container.image.split("@")[0]?.includes("/"),
			"The container image must be pinned to an exact digest.",
		);
		assert(
			Array.isArray(container.platforms) &&
				container.platforms.length > 0 &&
				container.platforms.length <= 2 &&
				container.platforms.every(
					(platform) =>
						platform === "linux/amd64" || platform === "linux/arm64",
				) &&
				new Set(container.platforms).size === container.platforms.length,
			"Invalid container platforms.",
		);
	}
	return value as unknown as StandaloneRelease;
}

export async function verifyReleaseManifest(
	manifestJws: string,
	input: ReleaseConfig,
	now = Math.floor(Date.now() / 1000),
): Promise<VerifiedRelease> {
	const config = validateReleaseConfig(input);
	const { payload, fingerprint } = await verifyJws(
		manifestJws,
		config.publicKeys,
		"flow-like-standalone-release+jws",
	);
	const manifest = validateManifest(payload, config, now);
	return {
		manifest,
		manifestJws,
		signerFingerprint: fingerprint,
		targets: manifest.artifacts.map((artifact) => artifact.target),
	};
}

function containerPlatform(target: ReleaseTarget): string | null {
	return target === "x86_64-unknown-linux-gnu"
		? "linux/amd64"
		: target === "aarch64-unknown-linux-gnu"
			? "linux/arm64"
			: null;
}

export function standalonePackageModes(
	release: StandaloneRelease,
	target: ReleaseTarget,
): { binary: boolean; docker: boolean } {
	const artifact = release.artifacts.find((value) => value.target === target);
	const platform = containerPlatform(target);
	return {
		binary: Boolean(artifact && artifact.size <= MAX_BROWSER_BINARY),
		docker: Boolean(
			artifact && platform && release.container?.platforms.includes(platform),
		),
	};
}

export function validateStandalonePackageSelection(
	release: StandaloneRelease,
	target: ReleaseTarget,
	mode: StandalonePackageInput["mode"],
): void {
	assert(
		TARGETS.includes(target) && ["binary", "docker", "both"].includes(mode),
		"Select a supported target and package mode.",
	);
	assert(
		release.artifacts.some((value) => value.target === target),
		"The signed release has no binary for the selected target.",
	);
	const modes = standalonePackageModes(release, target);
	if (mode !== "docker")
		assert(
			modes.binary,
			modes.docker
				? "This binary exceeds the browser package limit of 256 MiB. Select Docker Compose."
				: "This binary exceeds the browser package limit of 256 MiB. This target has no Docker alternative in the signed release.",
		);
	if (mode !== "binary")
		assert(
			modes.docker,
			"The signed release has no container for the selected target.",
		);
}

async function download(
	url: string,
	limit: number,
	signal?: AbortSignal,
	expectedSize?: number,
): Promise<Uint8Array<ArrayBuffer>> {
	signal?.throwIfAborted();
	https(url);
	const timeout = AbortSignal.timeout(120_000);
	const response = await fetch(url, {
		credentials: "omit",
		referrerPolicy: "no-referrer",
		redirect: "error",
		cache: "no-store",
		signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
	});
	assert(
		response.status === 200 && !response.redirected,
		"Release download failed without following redirects.",
	);
	const declared = response.headers.get("content-length");
	if (declared !== null)
		assert(
			/^\d+$/.test(declared) &&
				Number(declared) <= limit &&
				(expectedSize === undefined || Number(declared) === expectedSize),
			"Release download size does not match the signed manifest.",
		);
	const reader = response.body?.getReader();
	assert(reader, "Release download has no readable body.");
	const chunks: Uint8Array[] = [];
	let size = 0;
	try {
		while (true) {
			const next = await reader.read();
			if (next.done) break;
			size += next.value.length;
			assert(size <= limit, "Release download exceeds its size limit.");
			chunks.push(next.value);
		}
	} finally {
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
	assert(
		expectedSize === undefined || size === expectedSize,
		"Release download was truncated or changed size.",
	);
	const bytes = new Uint8Array(size);
	let offset = 0;
	for (const chunk of chunks) {
		bytes.set(chunk, offset);
		offset += chunk.length;
	}
	return bytes;
}

export async function fetchVerifiedRelease(
	config: ReleaseConfig,
	signal?: AbortSignal,
): Promise<VerifiedRelease> {
	const checked = validateReleaseConfig(config);
	const bytes = await download(checked.manifestUrl, MAX_JWS, signal);
	return verifyReleaseManifest(decoder.decode(bytes), checked);
}

interface ZipEntry {
	name: string;
	bytes: Uint8Array<ArrayBuffer>;
	mode: number;
}
const crcTable = Uint32Array.from({ length: 256 }, (_, index) => {
	let value = index;
	for (let bit = 0; bit < 8; bit++)
		value = (value >>> 1) ^ (value & 1 ? 0xedb88320 : 0);
	return value >>> 0;
});
function crc32(bytes: Uint8Array): number {
	let crc = 0xffffffff;
	for (const byte of bytes) {
		crc = (crc >>> 8) ^ (crcTable[(crc ^ byte) & 0xff] ?? 0);
	}
	return (crc ^ 0xffffffff) >>> 0;
}
function zip(entries: ZipEntry[]): Blob {
	const content: BlobPart[] = [];
	const central: BlobPart[] = [];
	let offset = 0;
	let centralSize = 0;
	for (const entry of entries) {
		const name = encoder.encode(entry.name);
		const checksum = crc32(entry.bytes);
		const local = new Uint8Array(30 + name.length);
		const view = new DataView(local.buffer);
		view.setUint32(0, 0x04034b50, true);
		view.setUint16(4, 20, true);
		view.setUint16(6, 0x800, true);
		view.setUint16(12, 33, true);
		view.setUint32(14, checksum, true);
		view.setUint32(18, entry.bytes.length, true);
		view.setUint32(22, entry.bytes.length, true);
		view.setUint16(26, name.length, true);
		local.set(name, 30);
		const directory = new Uint8Array(46 + name.length);
		const item = new DataView(directory.buffer);
		item.setUint32(0, 0x02014b50, true);
		item.setUint16(4, 0x0314, true);
		item.setUint16(6, 20, true);
		item.setUint16(8, 0x800, true);
		item.setUint16(14, 33, true);
		item.setUint32(16, checksum, true);
		item.setUint32(20, entry.bytes.length, true);
		item.setUint32(24, entry.bytes.length, true);
		item.setUint16(28, name.length, true);
		item.setUint32(38, entry.mode << 16, true);
		item.setUint32(42, offset, true);
		directory.set(name, 46);
		content.push(local, entry.bytes);
		central.push(directory);
		centralSize += directory.length;
		offset += local.length + entry.bytes.length;
	}
	const end = new Uint8Array(22);
	const view = new DataView(end.buffer);
	view.setUint32(0, 0x06054b50, true);
	view.setUint16(8, entries.length, true);
	view.setUint16(10, entries.length, true);
	view.setUint32(12, centralSize, true);
	view.setUint32(16, offset, true);
	return new Blob([...content, ...central, end], { type: "application/zip" });
}

export async function buildStandalonePackage(
	input: StandalonePackageInput,
): Promise<Blob> {
	const release = input.verifiedRelease
		? await verifyReleaseManifest(
				input.verifiedRelease.manifestJws,
				input.release,
			)
		: await fetchVerifiedRelease(input.release, input.signal);
	validateStandalonePackageSelection(
		release.manifest,
		input.target,
		input.mode,
	);
	const artifact = release.manifest.artifacts.find(
		(artifact) => artifact.target === input.target,
	);
	assert(artifact, "The signed release has no binary for the selected target.");
	const platform = containerPlatform(input.target);
	const supplied = record(input.manifest);
	const controller = record(supplied.controller_key);
	assert(
		typeof controller.x === "string",
		"The onboarding controller key is missing.",
	);
	const verified = await verifyJws(
		input.manifest_jws,
		[controller.x],
		"flow-like-device-onboarding+jwt",
	);
	const manifest = record(verified.payload);
	exact(manifest, [
		"version",
		"enrollment_id",
		"device_id",
		"owner_id",
		"name",
		"api_base_url",
		"bootstrap_key",
		"controller_key",
		"owner_invitation_key",
		"issued_at",
		"expires_at",
	]);
	assert(
		canonical(manifest) === canonical(supplied),
		"The onboarding manifest differs from its signed template.",
	);
	assert(
		Number(manifest.expires_at) > Date.now() / 1000 &&
			Array.isArray(input.bootstrap_secret) &&
			input.bootstrap_secret.length === 32 &&
			input.bootstrap_secret.every(
				(byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255,
			),
		"The enrollment package is expired or missing its bootstrap key.",
	);
	assert(
		typeof input.enrollment_token === "string" &&
			input.enrollment_token.length > 0 &&
			input.enrollment_token.length <= MAX_JWS,
		"Invalid enrollment token.",
	);
	const bootstrap = record(manifest.bootstrap_key);
	exact(bootstrap, ["kty", "crv", "x"]);
	assert(
		bootstrap.kty === "OKP" &&
			bootstrap.crv === "Ed25519" &&
			typeof bootstrap.x === "string",
		"Invalid bootstrap public key.",
	);
	const pkcs8 = new Uint8Array(48);
	pkcs8.set([
		0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70,
		0x04, 0x22, 0x04, 0x20,
	]);
	pkcs8.set(input.bootstrap_secret, 16);
	try {
		const key = await crypto.subtle.importKey("pkcs8", pkcs8, "Ed25519", true, [
			"sign",
		]);
		const exported = await crypto.subtle.exportKey("jwk", key);
		assert(
			exported.x === bootstrap.x,
			"The bootstrap secret does not match the signed enrollment.",
		);
	} finally {
		pkcs8.fill(0);
	}
	const entries: ZipEntry[] = [];
	const add = (name: string, value: string, mode = 0o100600) =>
		entries.push({ name, bytes: encoder.encode(value), mode });
	if (input.mode !== "docker") {
		const binary = await download(
			artifact.url,
			artifact.size,
			input.signal,
			artifact.size,
		);
		assert(
			(await sha256(binary)) === artifact.sha256,
			"The binary digest does not match the signed release.",
		);
		entries.push({
			name: "flow-like-standalone",
			bytes: binary,
			mode: 0o100700,
		});
		add(
			"start.sh",
			'#!/bin/sh\nset -eu\ncd -- "$(dirname -- "$0")"\nchmod 700 . ./flow-like-standalone\nchmod 600 .env onboarding.json release-trust.json release.jws 2>/dev/null || true\nif [ -f onboarding.json ]; then ./flow-like-standalone --state-dir ./state enroll .; fi\nif [ -f ./state/agent.env ]; then chmod 600 ./state/agent.env; fi\nexec ./flow-like-standalone --state-dir ./state run\n',
			0o100700,
		);
	}
	add(".gitignore", "*\n!.gitignore\n");
	add(
		"state/agent.env",
		"# Operator-owned agent configuration. Restart the agent after changes.\n# Use required when project owners must not access the agent or sibling projects.\nFLOW_LIKE_DEVICE_ISOLATION_POLICY=compatible\n# Strict Linux placements also need delegated CPU/memory/PID budgets and ext4 project quotas.\n# FLOW_LIKE_DEVICE_CGROUP_ROOT=/sys/fs/cgroup/flow-like-workloads\n# Artifact admission includes committed revisions and in-flight uploads, plus metadata.\n# FILES counts files and directories; each in-flight file reserves 34 entries until commit.\n# Default per-project FILES admits about 1900 files per upload with no retained revisions.\n# Limits do not remove revisions; lowering them blocks new uploads until usage fits.\nFLOW_LIKE_DEVICE_ARTIFACT_BYTES=68719476736\nFLOW_LIKE_DEVICE_ARTIFACT_FILES=262144\nFLOW_LIKE_DEVICE_ARTIFACT_REVISIONS=1024\nFLOW_LIKE_PROJECT_ARTIFACT_BYTES=17179869184\nFLOW_LIKE_PROJECT_ARTIFACT_FILES=65536\nFLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=128\n",
	);
	add(
		".env",
		"FLOW_LIKE_STANDALONE_STATE_DIR=./state\nFLOW_LIKE_PUBLISH_HOST=127.0.0.1\nFLOW_LIKE_SERVICE_PORT=8080\n# Durable host isolation policy: edit ./state/agent.env (native service and Docker).\n",
	);
	add(
		".env.example",
		"FLOW_LIKE_STANDALONE_STATE_DIR=./state\nFLOW_LIKE_PUBLISH_HOST=127.0.0.1\nFLOW_LIKE_SERVICE_PORT=8080\n# Durable host isolation policy: edit ./state/agent.env (native service and Docker).\n",
	);
	add(
		"onboarding.json",
		JSON.stringify({
			manifest,
			manifest_jws: input.manifest_jws,
			enrollment_token: input.enrollment_token,
			bootstrap_secret: input.bootstrap_secret,
		}),
	);
	add("release.jws", release.manifestJws);
	add(
		"release-trust.json",
		JSON.stringify({
			manifest_url: input.release.manifestUrl,
			public_keys: input.release.publicKeys,
			minimum_sequence: Math.max(
				input.release.minimumSequence,
				release.manifest.sequence,
			),
		}),
	);
	add(
		"platform.json",
		JSON.stringify({
			target: input.target,
			version: release.manifest.release_version,
			release_sequence: release.manifest.sequence,
		}),
	);
	if (input.mode !== "binary") {
		const image = release.manifest.container?.image;
		assert(image && platform, "Container metadata is missing.");
		add(
			"compose.yaml",
			`services:\n  agent:\n    image: ${image}\n    platform: ${platform}\n    restart: unless-stopped\n    user: "\${FLOW_LIKE_DEVICE_UID:?Run start-docker.sh}:\${FLOW_LIKE_DEVICE_GID:?Run start-docker.sh}"\n    working_dir: /package\n    env_file: .env\n    ports:\n      - "\${FLOW_LIKE_PUBLISH_HOST:-127.0.0.1}:\${FLOW_LIKE_SERVICE_PORT:-8080}:\${FLOW_LIKE_SERVICE_PORT:-8080}"\n    volumes:\n      - ./:/package\n    entrypoint: ["/bin/sh", "/package/container-start.sh"]\n    stop_grace_period: 45s\n`,
		);
		add(
			"container-start.sh",
			"#!/bin/sh\nset -eu\nif [ -f /package/onboarding.json ]; then flow-like-standalone --state-dir /package/state enroll /package; fi\nexec flow-like-standalone --state-dir /package/state run\n",
			0o100700,
		);
		add(
			"start-docker.sh",
			'#!/bin/sh\nset -eu\ncd -- "$(dirname -- "$0")"\nchmod 700 .\nchmod 600 .env onboarding.json release-trust.json release.jws 2>/dev/null || true\nif [ -f ./state/agent.env ]; then chmod 600 ./state/agent.env; fi\nexport FLOW_LIKE_DEVICE_UID="$(id -u)"\nexport FLOW_LIKE_DEVICE_GID="$(id -g)"\nexec docker compose up -d\n',
			0o100700,
		);
	}
	input.signal?.throwIfAborted();
	assert(
		release.manifest.expires_at > Date.now() / 1000 &&
			Number(manifest.expires_at) > Date.now() / 1000,
		"Enrollment or release metadata expired while preparing the package.",
	);
	return zip(entries);
}
