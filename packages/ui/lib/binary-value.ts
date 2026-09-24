export type BinaryCompression = "gzip" | "deflate";

export type BinaryFormat =
	| "gzip"
	| "zlib"
	| "zstd"
	| "png"
	| "jpeg"
	| "gif"
	| "webp"
	| "pdf"
	| "zip"
	| "parquet";

export interface BinaryDecodeOptions {
	/** Stored payloads larger than this are left undecoded. */
	maxBytes: number;
	/** Decompression past this many bytes gives up and keeps the payload binary. */
	maxDecodedBytes: number;
	/** Text longer than this is cut, for surfaces that show only a preview. */
	maxTextLength?: number;
	/** Pretty-prints JSON so the detail view need not re-serialize it. */
	prettyJson?: boolean;
}

/** A table cell shows one line, so it decodes small payloads and keeps a snippet. */
export const CELL_DECODE_OPTIONS: BinaryDecodeOptions = {
	maxBytes: 256 * 1024,
	maxDecodedBytes: 2 * 1024 * 1024,
	maxTextLength: 240,
};

export const DETAIL_DECODE_OPTIONS: BinaryDecodeOptions = {
	maxBytes: 16 * 1024 * 1024,
	maxDecodedBytes: 32 * 1024 * 1024,
	prettyJson: true,
};

/** Bytes kept from a payload that stays binary, enough for a hex preview. */
export const BINARY_HEAD_BYTES = 512;

interface ReadingBase {
	byteLength: number;
	compression?: BinaryCompression;
}

export type BinaryReading =
	| (ReadingBase & {
			kind: "json";
			text: string;
			decodedLength: number;
			shape: "object" | "array";
			entries: number;
			truncated: boolean;
	  })
	| (ReadingBase & {
			kind: "text";
			text: string;
			decodedLength: number;
			truncated: boolean;
	  })
	| (ReadingBase & {
			kind: "binary";
			reason: "not-text" | "too-large";
			format?: BinaryFormat;
			/** The first bytes of the payload, inflated when it was compressed. */
			head: Uint8Array;
			payloadLength: number;
	  });

/**
 * The bytes a cell holds. Binary columns reach the browser as JSON arrays of
 * octets, so an array counts only when every element is one.
 */
export function binaryBytes(value: unknown): Uint8Array | null {
	if (value instanceof Uint8Array) return value;
	if (value instanceof ArrayBuffer) return new Uint8Array(value);
	if (!Array.isArray(value)) return null;
	for (const item of value) {
		if (!Number.isInteger(item) || item < 0 || item > 255) return null;
	}
	return Uint8Array.from(value as number[]);
}

export function binaryByteLength(value: unknown): number {
	if (value instanceof Uint8Array || value instanceof ArrayBuffer) {
		return value.byteLength;
	}
	return Array.isArray(value) ? value.length : 0;
}

const startsWith = (bytes: Uint8Array, signature: readonly number[], at = 0) =>
	bytes.length >= at + signature.length &&
	signature.every((byte, index) => bytes[at + index] === byte);

const ascii = (text: string) => Array.from(text, (char) => char.charCodeAt(0));

const SIGNATURES: readonly [BinaryFormat, readonly number[]][] = [
	["gzip", [0x1f, 0x8b]],
	["zstd", [0x28, 0xb5, 0x2f, 0xfd]],
	["png", [0x89, 0x50, 0x4e, 0x47]],
	["jpeg", [0xff, 0xd8, 0xff]],
	["gif", ascii("GIF8")],
	["pdf", ascii("%PDF")],
	["zip", [0x50, 0x4b, 0x03, 0x04]],
	["parquet", ascii("PAR1")],
];

/** A zlib header is a CM=8 byte whose 16-bit value with the flags is a multiple of 31. */
const isZlibHeader = (bytes: Uint8Array) =>
	bytes.length >= 2 &&
	(bytes[0] & 0x0f) === 8 &&
	bytes[0] >> 4 <= 7 &&
	((bytes[0] << 8) | bytes[1]) % 31 === 0;

export function sniffBinaryFormat(bytes: Uint8Array): BinaryFormat | undefined {
	for (const [format, signature] of SIGNATURES) {
		if (startsWith(bytes, signature)) return format;
	}
	if (startsWith(bytes, ascii("RIFF")) && startsWith(bytes, ascii("WEBP"), 8)) {
		return "webp";
	}
	return isZlibHeader(bytes) ? "zlib" : undefined;
}

const COMPRESSION_BY_FORMAT: Partial<Record<BinaryFormat, BinaryCompression>> =
	{ gzip: "gzip", zlib: "deflate" };

type Inflated = Uint8Array | "too-large" | "invalid";

async function inflate(
	bytes: Uint8Array,
	compression: BinaryCompression,
	maxDecodedBytes: number,
): Promise<Inflated> {
	const source = new ReadableStream<BufferSource>({
		start(controller) {
			controller.enqueue(bytes as BufferSource);
			controller.close();
		},
	});
	const reader = source
		.pipeThrough(new DecompressionStream(compression))
		.getReader();
	const chunks: Uint8Array[] = [];
	let total = 0;
	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (done) break;
			total += value.byteLength;
			if (total > maxDecodedBytes) {
				await reader.cancel().catch(() => undefined);
				return "too-large";
			}
			chunks.push(value);
		}
	} catch {
		return "invalid";
	}
	const out = new Uint8Array(total);
	let offset = 0;
	for (const chunk of chunks) {
		out.set(chunk, offset);
		offset += chunk.byteLength;
	}
	return out;
}

/**
 * Valid UTF-8 can still be binary: a NUL or a run of control characters
 * does not occur in text anyone wrote.
 */
export function looksLikeText(text: string): boolean {
	let control = 0;
	for (let index = 0; index < text.length; index++) {
		const code = text.charCodeAt(index);
		if (code === 0) return false;
		if (code < 0x20 && code !== 0x09 && code !== 0x0a && code !== 0x0d) {
			control++;
		} else if (code === 0x7f) {
			control++;
		}
	}
	return control <= Math.max(1, text.length * 0.01);
}

const utf8 = (bytes: Uint8Array): string | null => {
	try {
		return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
	} catch {
		return null;
	}
};

const clip = (text: string, maxLength?: number) =>
	maxLength !== undefined && text.length > maxLength
		? { text: text.slice(0, maxLength), truncated: true }
		: { text, truncated: false };

/** `payload` is what the preview shows; the size reported is always the stored one. */
const binaryReading = (
	payload: Uint8Array,
	reason: "not-text" | "too-large",
	byteLength: number,
	compression?: BinaryCompression,
): Extract<BinaryReading, { kind: "binary" }> => ({
	kind: "binary",
	reason,
	byteLength,
	format: sniffBinaryFormat(payload),
	head: payload.slice(0, BINARY_HEAD_BYTES),
	payloadLength: payload.byteLength,
	...(compression ? { compression } : {}),
});

/**
 * The reading of a payload too large to decode, built from its first bytes so a
 * multi-megabyte octet array is never copied just to be refused.
 */
export function oversizedBinaryReading(
	value: unknown,
	options: BinaryDecodeOptions,
): BinaryReading | null {
	const byteLength = binaryByteLength(value);
	if (byteLength <= options.maxBytes) return null;
	const head = binaryBytes(
		Array.isArray(value) ? value.slice(0, BINARY_HEAD_BYTES) : value,
	);
	return head
		? {
				...binaryReading(head, "too-large", byteLength),
				payloadLength: byteLength,
			}
		: null;
}

function readText(
	text: string,
	byteLength: number,
	decodedLength: number,
	options: BinaryDecodeOptions,
	compression?: BinaryCompression,
): BinaryReading {
	const base = {
		byteLength,
		decodedLength,
		...(compression ? { compression } : {}),
	};
	const first = text.trimStart()[0];
	if (first === "{" || first === "[") {
		try {
			const json: unknown = JSON.parse(text);
			const isArray = Array.isArray(json);
			return {
				kind: "json",
				...base,
				shape: isArray ? "array" : "object",
				entries: isArray
					? json.length
					: Object.keys(json as Record<string, unknown>).length,
				...clip(
					options.prettyJson ? JSON.stringify(json, null, 2) : text,
					options.maxTextLength,
				),
			};
		} catch {
			// Braces that do not parse are still readable text.
		}
	}
	return { kind: "text", ...base, ...clip(text, options.maxTextLength) };
}

/**
 * Reads stored bytes as the richest thing they hold: JSON, then text, then
 * raw bytes. Gzip and zlib payloads are inflated first, within a size budget
 * so a small cell cannot expand into an unbounded allocation.
 */
export async function decodeBinary(
	bytes: Uint8Array,
	options: BinaryDecodeOptions,
): Promise<BinaryReading> {
	const { byteLength } = bytes;
	if (byteLength > options.maxBytes) {
		return binaryReading(bytes, "too-large", byteLength);
	}

	const format = sniffBinaryFormat(bytes);
	let compression = format ? COMPRESSION_BY_FORMAT[format] : undefined;
	let payload = bytes;
	if (compression && typeof DecompressionStream !== "undefined") {
		const inflated = await inflate(bytes, compression, options.maxDecodedBytes);
		if (inflated === "too-large") {
			return binaryReading(bytes, "too-large", byteLength, compression);
		}
		// A lookalike header over bytes that do not inflate is not compression.
		if (inflated === "invalid") compression = undefined;
		else payload = inflated;
	} else {
		compression = undefined;
	}

	const text = utf8(payload);
	if (text === null || !looksLikeText(text)) {
		return binaryReading(payload, "not-text", byteLength, compression);
	}
	return readText(text, byteLength, payload.byteLength, options, compression);
}

/** The classic `offset  hex bytes  |ascii|` layout, sixteen bytes a line. */
export function formatHexDump(bytes: Uint8Array, bytesPerLine = 16): string {
	const lines: string[] = [];
	for (let offset = 0; offset < bytes.length; offset += bytesPerLine) {
		const line = bytes.subarray(offset, offset + bytesPerLine);
		const hex = Array.from(line, (byte) => byte.toString(16).padStart(2, "0"))
			.join(" ")
			.padEnd(bytesPerLine * 3 - 1, " ");
		const text = Array.from(line, (byte) =>
			byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : ".",
		).join("");
		lines.push(`${offset.toString(16).padStart(8, "0")}  ${hex}  |${text}|`);
	}
	return lines.join("\n");
}

export function bytesToBase64(bytes: Uint8Array): string {
	let binary = "";
	const chunk = 0x8000;
	for (let offset = 0; offset < bytes.length; offset += chunk) {
		binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
	}
	return btoa(binary);
}
