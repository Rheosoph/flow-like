/**
 * How a stored board snapshot is encoded. `gzip-base64` is the normal case; `json` is
 * written when compression is unavailable or the page is unloading and there is no time.
 * Base64 rather than raw bytes: the desktop's IndexedDB is a SQLite shim that typeson-walks
 * typed arrays and sends values over JSON IPC, where a string is the cheapest thing to carry.
 */
export type SnapshotEncoding = "gzip-base64" | "json";

export interface PackedSnapshot {
	payload: string;
	encoding: SnapshotEncoding;
}

const BASE64_CHUNK = 0x8000;

function bytesToBase64(bytes: Uint8Array): string {
	let binary = "";
	for (let i = 0; i < bytes.length; i += BASE64_CHUNK) {
		binary += String.fromCharCode(...bytes.subarray(i, i + BASE64_CHUNK));
	}
	return btoa(binary);
}

function base64ToBytes(base64: string): Uint8Array {
	const binary = atob(base64);
	const bytes = new Uint8Array(binary.length);
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

async function pipe(
	bytes: Uint8Array,
	transform: CompressionStream | DecompressionStream,
): Promise<Uint8Array> {
	const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(transform);
	return new Uint8Array(await new Response(stream).arrayBuffer());
}

export const canCompress = () =>
	typeof CompressionStream !== "undefined" &&
	typeof DecompressionStream !== "undefined";

export async function packSnapshot(json: string): Promise<PackedSnapshot> {
	if (!canCompress()) return { payload: json, encoding: "json" };
	const zipped = await pipe(
		new TextEncoder().encode(json),
		new CompressionStream("gzip"),
	);
	return { payload: bytesToBase64(zipped), encoding: "gzip-base64" };
}

export async function unpackSnapshot(packed: PackedSnapshot): Promise<string> {
	if (packed.encoding === "json") return packed.payload;
	if (!canCompress()) {
		throw new Error(
			"Reading a gzip board snapshot failed: DecompressionStream is not available here",
		);
	}
	const bytes = await pipe(
		base64ToBytes(packed.payload),
		new DecompressionStream("gzip"),
	);
	return new TextDecoder().decode(bytes);
}

/** cyrb53: a fast 53-bit string hash, enough to tell "same board text" from "changed". */
export function fingerprint(text: string): string {
	let h1 = 0xdeadbeef;
	let h2 = 0x41c6ce57;
	for (let i = 0; i < text.length; i++) {
		const ch = text.charCodeAt(i);
		h1 = Math.imul(h1 ^ ch, 2654435761);
		h2 = Math.imul(h2 ^ ch, 1597334677);
	}
	h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507);
	h1 ^= Math.imul(h2 ^ (h2 >>> 13), 3266489909);
	h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507);
	h2 ^= Math.imul(h1 ^ (h1 >>> 13), 3266489909);
	return `${text.length}:${(4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(36)}`;
}
