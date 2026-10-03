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
export type ReleaseCheck =
	| "signature"
	| "sequence"
	| "not_yet_valid"
	| "expired"
	| "lifetime"
	| "invalid";
/** What a genuinely signed release list says about itself. */
export interface ReleaseFacts {
	release_version: string;
	sequence: number;
	issued_at: number;
	expires_at: number;
}
/**
 * The release list arrived and failed one check. Screens read `check`; the
 * message stays English for diagnostics. `facts` is set only for checks that
 * run after the signature verified.
 */
export class ReleaseVerificationError extends Error {
	readonly check: ReleaseCheck;
	readonly facts?: ReleaseFacts;

	constructor(check: ReleaseCheck, message: string, facts?: ReleaseFacts) {
		super(message);
		this.name = "ReleaseVerificationError";
		this.check = check;
		if (facts) this.facts = facts;
	}
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
export const MAX_RELEASE_LIFETIME_S = 1_825 * 86_400;
export const RELEASE_NOT_YET_VALID_TOLERANCE_S = 300;
const MAX_JWS = 16 * 1024;
const MAX_BROWSER_BINARY = 256 * 1024 * 1024;
const MAX_RELEASE_BINARY = 2 * 1024 ** 3;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const fail = (message: string): never => {
	throw new Error(message);
};
/** No trusted key signed these bytes. */
class UntrustedSignature extends Error {}
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
		const genuine = await crypto.subtle.verify(
			"Ed25519",
			key,
			signature,
			encoder.encode(`${encodedHeader}.${encodedPayload}`),
		);
		if (!genuine)
			throw new UntrustedSignature("Release signature verification failed.");
		return {
			payload: strictJson(decoder.decode(decode(encodedPayload))),
			fingerprint,
		};
	}
	throw new UntrustedSignature(
		"The package signature does not match a configured release key.",
	);
}

const RELEASE_VERSION =
	/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][A-Za-z\d-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][A-Za-z\d-]*))*)?(?:\+[A-Za-z\d-]+(?:\.[A-Za-z\d-]+)*)?$/;
const CONTAINER_PLATFORMS: readonly unknown[] = ["linux/amd64", "linux/arm64"];

function validateReleaseNumbers(value: Record<string, unknown>) {
	assert(
		value.version === 1 &&
			Number.isSafeInteger(value.state_schema_version) &&
			Number(value.state_schema_version) > 0 &&
			Number(value.state_schema_version) <= 0xffffffff &&
			Number.isSafeInteger(value.sequence) &&
			Number(value.sequence) > 0,
		"Invalid release version, schema version or sequence.",
	);
}

function validateReleaseVersion(version: unknown) {
	assert(
		typeof version === "string" &&
			version.length <= 64 &&
			RELEASE_VERSION.test(version),
		"Invalid standalone release version.",
	);
	assert(
		version
			.split(/[-+]/)[0]
			?.split(".")
			.every((part) => BigInt(part) <= 18_446_744_073_709_551_615n),
		"Release version component exceeds its limit.",
	);
}

function validateReleaseDates(value: Record<string, unknown>) {
	assert(
		Number.isSafeInteger(value.issued_at) &&
			Number.isSafeInteger(value.expires_at) &&
			Number(value.issued_at) >= 0 &&
			Number(value.expires_at) > Number(value.issued_at),
		"Invalid release validity dates.",
	);
}

function validateArtifact(item: unknown, targets: Set<unknown>) {
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
			Number(artifact.size) <= MAX_RELEASE_BINARY &&
			typeof artifact.sha256 === "string" &&
			/^[a-f0-9]{64}$/.test(artifact.sha256),
		"Invalid release artifact digest or size.",
	);
}

function validateArtifacts(artifacts: unknown) {
	assert(
		Array.isArray(artifacts) && artifacts.length > 0 && artifacts.length <= 4,
		"Invalid release artifact count.",
	);
	const targets = new Set<unknown>();
	for (const item of artifacts) validateArtifact(item, targets);
}

function validPlatforms(platforms: unknown) {
	return (
		Array.isArray(platforms) &&
		platforms.length > 0 &&
		platforms.length <= 2 &&
		platforms.every((platform) => CONTAINER_PLATFORMS.includes(platform)) &&
		new Set(platforms).size === platforms.length
	);
}

function validateContainer(value: unknown) {
	if (value === null) return;
	const container = record(value);
	exact(container, ["image", "platforms"]);
	assert(
		typeof container.image === "string" &&
			/^[a-z0-9][a-z0-9/._:-]{1,254}@sha256:[a-f0-9]{64}$/.test(
				container.image,
			) &&
			container.image.split("@")[0]?.includes("/"),
		"The container image must be pinned to an exact digest.",
	);
	assert(validPlatforms(container.platforms), "Invalid container platforms.");
}

/** The shape of a release list, whatever the clock and the hub's minimum say. */
function validateManifest(input: unknown): StandaloneRelease {
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
	validateReleaseNumbers(value);
	validateReleaseVersion(value.release_version);
	validateReleaseDates(value);
	validateArtifacts(value.artifacts);
	validateContainer(value.container);
	return value as unknown as StandaloneRelease;
}

/** Every failure after the list arrived is a failed check; nothing else reaches the caller. */
function releaseFailure(error: unknown) {
	if (error instanceof ReleaseVerificationError) return error;
	return new ReleaseVerificationError(
		error instanceof UntrustedSignature ? "signature" : "invalid",
		error instanceof Error ? error.message : String(error),
	);
}

/** A well-formed, genuinely signed list against the hub's minimum and the clock. The first failing check is the reason. */
function checkRelease(
	release: StandaloneRelease,
	config: ReleaseConfig,
	now: number,
) {
	const facts: ReleaseFacts = {
		release_version: release.release_version,
		sequence: release.sequence,
		issued_at: release.issued_at,
		expires_at: release.expires_at,
	};
	const refuse = (check: ReleaseCheck, message: string): never => {
		throw new ReleaseVerificationError(check, message, facts);
	};
	if (release.sequence < config.minimumSequence)
		refuse(
			"sequence",
			"The release sequence is older than the configured minimum.",
		);
	if (release.expires_at - release.issued_at > MAX_RELEASE_LIFETIME_S)
		refuse(
			"lifetime",
			"The release manifest is valid for longer than this app accepts.",
		);
	if (release.issued_at > now + RELEASE_NOT_YET_VALID_TOLERANCE_S)
		refuse("not_yet_valid", "The release manifest is not valid yet.");
	if (release.expires_at <= now)
		refuse("expired", "The release manifest has expired.");
}

export async function verifyReleaseManifest(
	manifestJws: string,
	input: ReleaseConfig,
	now = Math.floor(Date.now() / 1000),
): Promise<VerifiedRelease> {
	const config = validateReleaseConfig(input);
	try {
		const { payload, fingerprint } = await verifyJws(
			manifestJws,
			config.publicKeys,
			"flow-like-standalone-release+jws",
		);
		const manifest = validateManifest(payload);
		checkRelease(manifest, config, now);
		return {
			manifest,
			manifestJws,
			signerFingerprint: fingerprint,
			targets: manifest.artifacts.map((artifact) => artifact.target),
		};
	} catch (error) {
		throw releaseFailure(error);
	}
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
		binary: Boolean(artifact && artifact.size <= MAX_RELEASE_BINARY),
		docker: Boolean(
			artifact && platform && release.container?.platforms.includes(platform),
		),
	};
}

export function standalonePackageDownloadsBinary(
	release: StandaloneRelease,
	target: ReleaseTarget,
): boolean {
	const artifact = release.artifacts.find((value) => value.target === target);
	return Boolean(artifact && artifact.size > MAX_BROWSER_BINARY);
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
			"This binary exceeds the release size limit of 2 GiB.",
		);
	if (mode !== "binary")
		assert(
			modes.docker,
			"The signed release has no container for the selected target.",
		);
}

function binaryDownloadScript(artifact: StandaloneArtifact): string {
	const quotedUrl = `'${artifact.url.replaceAll("'", `'"'"'`)}'`;
	return `#!/bin/sh
set -eu
umask 077
export LC_ALL=C
cd -- "$(dirname -- "$0")"
chmod 700 .
if [ -L ./flow-like-standalone ] || { [ -e ./flow-like-standalone ] && [ ! -f ./flow-like-standalone ]; }; then
  echo 'The runtime destination must be a regular file.' >&2
  exit 1
fi
# An enrolled runtime may have installed a newer signed release through its updater.
if [ -f ./flow-like-standalone ] && [ ! -f onboarding.json ]; then exit 0; fi
if command -v sha256sum >/dev/null 2>&1; then
  checksum() { sha256sum "$1"; }
elif command -v shasum >/dev/null 2>&1; then
  checksum() { shasum -a 256 "$1"; }
else
  echo 'Install sha256sum or shasum to verify the runtime.' >&2
  exit 1
fi
verify_binary() {
  [ "$(wc -c < "$1")" -eq ${artifact.size} ] || { echo 'Runtime size differs from the signed release.' >&2; return 1; }
  digest=$(checksum "$1") || return 1
  [ "\${digest%% *}" = '${artifact.sha256}' ] || { echo 'Runtime digest differs from the signed release.' >&2; return 1; }
}
if [ -f ./flow-like-standalone ]; then verify_binary ./flow-like-standalone; exit 0; fi
command -v curl >/dev/null 2>&1 || { echo 'Install curl to download the runtime.' >&2; exit 1; }
download_dir=$(mktemp -d ./.runtime-download.XXXXXX)
trap 'rm -rf "$download_dir"' 0
trap 'exit 1' HUP INT TERM
echo 'Downloading and verifying the native runtime...'
# Older curl versions cannot limit unknown-length responses. Shell file limits
# use 512 or 1024 byte blocks, so this also bounds disk use to at most twice the signed size.
status=$(
  ulimit -f ${Math.ceil(artifact.size / 512)} || exit 1
  curl --disable --fail --silent --show-error --globoff \\
    --proto '=https' --max-redirs 0 --connect-timeout 30 \\
    --max-time ${300 + Math.floor(artifact.size / (32 * 1024))} --speed-limit 1 --speed-time 60 \\
    --max-filesize ${artifact.size} --output "$download_dir/binary" \\
    --write-out '%{http_code}' --url ${quotedUrl}
)
[ "$status" = 200 ] || { echo 'Runtime download requires a direct HTTP 200 response.' >&2; exit 1; }
verify_binary "$download_dir/binary"
chmod 700 "$download_dir/binary"
mv -f "$download_dir/binary" ./flow-like-standalone
`;
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
		const deferredDownload = artifact.size > MAX_BROWSER_BINARY;
		if (deferredDownload) {
			add("download-runtime.sh", binaryDownloadScript(artifact), 0o100700);
		} else {
			const binary = await download(
				artifact.url,
				MAX_BROWSER_BINARY,
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
		}
		add(
			"start.sh",
			`#!/bin/sh
set -eu
cd -- "$(dirname -- "$0")"
${deferredDownload ? "sh ./download-runtime.sh\n" : ""}chmod 700 . ./flow-like-standalone
chmod 600 .env onboarding.json release-trust.json release.jws 2>/dev/null || true
if [ -f onboarding.json ]; then ./flow-like-standalone --state-dir ./state enroll .; fi
if [ -f ./state/agent.env ]; then chmod 600 ./state/agent.env; fi
exec ./flow-like-standalone --state-dir ./state run
`,
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
