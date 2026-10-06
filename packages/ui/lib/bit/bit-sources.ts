import type { IBit } from "../schema/bit/bit";

/*
 * Mirror of `bit_sources()` and `Bit::content_digest()` in
 * packages/core/runtime/src/bit.rs (plan §3.1). Both answer every case of
 * packages/core/runtime/fixtures/bit-sources.json alike, so a deployer and the
 * UI name the same sources and digests for a Bit.
 */

export interface BitContentDigest {
	algorithm: "blake3" | "sha256";
	hex: string;
}

const HUGGING_FACE = "https://huggingface.co";
const MAX_SOURCE_LENGTH = 2048;
const MAX_PATH_BYTES = 1024;
const MAX_PATH_COMPONENTS = 64;
const MAX_COMPONENT_BYTES = 255;
const HEX_DIGEST = /^[0-9a-f]{64}$/iu;
const REVISION = /^[0-9a-f]{40,64}$/iu;
const REPO_COMPONENT = /^[A-Za-z0-9](?:[A-Za-z0-9._-]{0,94}[A-Za-z0-9])?$/u;
const CONTROL = /\p{Cc}/u;
const encoder = new TextEncoder();
/** What the url crate percent-encodes in a pushed path segment of an `https` URL, besides controls and non-ASCII. */
const SEGMENT_ENCODED = new Set(
	[...' "<>`#?{}/%\\'].map((char) => char.charCodeAt(0)),
);

function encodeSegment(segment: string): string {
	let encoded = "";
	for (const byte of encoder.encode(segment))
		encoded +=
			byte < 0x20 || byte >= 0x7f || SEGMENT_ENCODED.has(byte)
				? `%${byte.toString(16).toUpperCase().padStart(2, "0")}`
				: String.fromCharCode(byte);
	return encoded;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A repository-relative path a Hugging Face resolve URL can name (`safe_mlx_asset_path`). */
function safeRepoPath(fileName: string): boolean {
	const parts = fileName.split("/");
	return (
		fileName.length > 0 &&
		encoder.encode(fileName).length <= MAX_PATH_BYTES &&
		!fileName.includes("\\") &&
		!CONTROL.test(fileName) &&
		parts.length <= MAX_PATH_COMPONENTS &&
		parts.every(
			(part) =>
				part !== "" &&
				part !== "." &&
				part !== ".." &&
				!part.includes(":") &&
				encoder.encode(part).length <= MAX_COMPONENT_BYTES,
		)
	);
}

/** The repository and commit a Bit's provider names, when they pin a Hugging Face file. */
function huggingFaceOrigin(
	bit: IBit,
): { repo: string; revision: string } | undefined {
	const provider: unknown = bit.parameters?.provider;
	if (!isRecord(provider)) return undefined;
	const { model_id: repo, version: revision } = provider;
	if (typeof repo !== "string" || typeof revision !== "string")
		return undefined;
	const parts = repo.split("/");
	if (parts.length !== 2 || !parts.every((part) => REPO_COMPONENT.test(part)))
		return undefined;
	return REVISION.test(revision)
		? { repo, revision: revision.toLowerCase() }
		: undefined;
}

function huggingFaceSource(
	origin: { repo: string; revision: string },
	fileName: string,
): string | undefined {
	if (!safeRepoPath(fileName)) return undefined;
	const path = fileName.split("/").map(encodeSegment).join("/");
	const source = `${HUGGING_FACE}/${origin.repo}/resolve/${origin.revision}/${path}?download=true`;
	return source.length <= MAX_SOURCE_LENGTH ? source : undefined;
}

function parsedUrl(link: string) {
	try {
		return new URL(link);
	} catch {
		return undefined;
	}
}

/** HTTPS without credentials, fragment or a query other than `download=true`; an empty `?` or `#` counts, as the url crate reads it. */
function isPublicHttps(url: URL, link: string) {
	const query = link.includes("?") ? url.search.slice(1) : null;
	return (
		url.protocol === "https:" &&
		url.hostname !== "" &&
		!url.username &&
		!url.password &&
		!link.includes("#") &&
		(query === null || query === "download=true")
	);
}

/** A link a device may fetch: public HTTPS in canonical form. */
export function publicSource(link: unknown) {
	if (typeof link !== "string" || link.length > MAX_SOURCE_LENGTH)
		return undefined;
	const url = parsedUrl(link);
	return url && isPublicHttps(url, link) && url.href === link
		? link
		: undefined;
}

/**
 * Where a device fetches a Bit's file, in the order it tries them: the Bit's
 * own public link, the file at the Hugging Face commit the Bit names, then at
 * the commit its pack `root` names. A Bit without a file has none.
 */
export function bitSources(bit: IBit, root: IBit = bit): string[] {
	const fileName = bit.file_name;
	if (typeof fileName !== "string" || !fileName) return [];
	const derived = [bit, root].flatMap((origin) => {
		const pinned = huggingFaceOrigin(origin);
		const source = pinned && huggingFaceSource(pinned, fileName);
		return source ? [source] : [];
	});
	const own = publicSource(bit.download_link);
	return [...new Set([...(own ? [own] : []), ...derived])];
}

/**
 * The digest of a Bit file's bytes known before any byte is fetched: the hub's
 * blake3 of a mirrored file (its `hash`), else the Git LFS sha256 the pack
 * root's Hugging Face manifest records for it. `undefined` leaves the digest to
 * the deployer, which hashes the file itself.
 */
export function bitContentDigest(
	bit: IBit,
	root: IBit = bit,
): BitContentDigest | undefined {
	const fileName = bit.file_name;
	if (typeof fileName !== "string" || !fileName) return undefined;
	if (HEX_DIGEST.test(bit.hash) && bit.hash !== bit.id)
		return { algorithm: "blake3", hex: bit.hash.toLowerCase() };
	const manifest: unknown = root.parameters?.huggingface;
	const files = isRecord(manifest) ? manifest.files : undefined;
	if (!Array.isArray(files)) return undefined;
	const entry: unknown = files.find(
		(file) => isRecord(file) && file.path === fileName,
	);
	const oid = isRecord(entry) ? entry.lfs_oid : undefined;
	return typeof oid === "string" && HEX_DIGEST.test(oid)
		? { algorithm: "sha256", hex: oid.toLowerCase() }
		: undefined;
}
