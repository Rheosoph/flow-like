import { describe, expect, test } from "bun:test";
import { deflateSync, gzipSync } from "node:zlib";
import {
	BINARY_HEAD_BYTES,
	type BinaryDecodeOptions,
	CELL_DECODE_OPTIONS,
	DETAIL_DECODE_OPTIONS,
	binaryBytes,
	bytesToBase64,
	decodeBinary,
	formatHexDump,
	looksLikeText,
	oversizedBinaryReading,
	sniffBinaryFormat,
} from "./binary-value";

const utf8 = (text: string) => new TextEncoder().encode(text);
const gzip = (text: string) => new Uint8Array(gzipSync(utf8(text)));

const FEATURE = {
	hex: "406544",
	flight: "DLH4AB ",
	altitude: 37000,
	squawk: "1000",
};

describe("binaryBytes", () => {
	test("reads the octet arrays binary columns arrive as", () => {
		expect(binaryBytes([31, 139, 0, 255])).toEqual(
			new Uint8Array([31, 139, 0, 255]),
		);
		expect(binaryBytes([])).toEqual(new Uint8Array());
	});

	test("refuses arrays that are not octets", () => {
		expect(binaryBytes([1, 256])).toBeNull();
		expect(binaryBytes([1.5])).toBeNull();
		expect(binaryBytes(["1"])).toBeNull();
		expect(binaryBytes("31,139")).toBeNull();
		expect(binaryBytes(null)).toBeNull();
	});
});

describe("sniffBinaryFormat", () => {
	test("recognizes compressed and common file payloads by their magic bytes", () => {
		expect(sniffBinaryFormat(gzip("{}"))).toBe("gzip");
		expect(sniffBinaryFormat(new Uint8Array(deflateSync(utf8("{}"))))).toBe(
			"zlib",
		);
		expect(
			sniffBinaryFormat(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a])),
		).toBe("png");
		expect(sniffBinaryFormat(utf8("%PDF-1.7"))).toBe("pdf");
		expect(sniffBinaryFormat(utf8("RIFF\0\0\0\0WEBPVP8 "))).toBe("webp");
	});

	test("leaves ordinary text unrecognized", () => {
		expect(sniffBinaryFormat(utf8('{"a":1}'))).toBeUndefined();
		expect(sniffBinaryFormat(utf8("hello"))).toBeUndefined();
	});
});

describe("decodeBinary", () => {
	test("inflates gzip and parses the JSON inside", async () => {
		const text = JSON.stringify(FEATURE);
		const stored = gzip(text);
		const reading = await decodeBinary(stored, DETAIL_DECODE_OPTIONS);

		expect(reading).toMatchObject({
			kind: "json",
			compression: "gzip",
			shape: "object",
			entries: 4,
			byteLength: stored.byteLength,
			decodedLength: utf8(text).byteLength,
			truncated: false,
		});
		if (reading.kind !== "json") throw new Error("expected JSON");
		expect(JSON.parse(reading.text)).toEqual(FEATURE);
		expect(reading.text).toContain('\n  "hex": "406544"');
	});

	test("inflates zlib streams too", async () => {
		const stored = new Uint8Array(deflateSync(utf8("[1,2,3]")));
		expect(await decodeBinary(stored, CELL_DECODE_OPTIONS)).toMatchObject({
			kind: "json",
			compression: "deflate",
			shape: "array",
			entries: 3,
		});
	});

	test("reads uncompressed UTF-8 as text, keeping only a snippet for cells", async () => {
		const text = `größe ${"x".repeat(500)}`;
		const reading = await decodeBinary(utf8(text), CELL_DECODE_OPTIONS);
		expect(reading).toMatchObject({ kind: "text", truncated: true });
		if (reading.kind !== "text") throw new Error("expected text");
		expect(reading.compression).toBeUndefined();
		expect(reading.text).toBe(text.slice(0, CELL_DECODE_OPTIONS.maxTextLength));
	});

	test("keeps braces that do not parse as text", async () => {
		expect(
			await decodeBinary(utf8("{not json"), CELL_DECODE_OPTIONS),
		).toMatchObject({ kind: "text", text: "{not json" });
	});

	test("keeps invalid UTF-8 and control-heavy bytes binary", async () => {
		const png = new Uint8Array([
			0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13,
		]);
		expect(await decodeBinary(png, CELL_DECODE_OPTIONS)).toMatchObject({
			kind: "binary",
			reason: "not-text",
			format: "png",
			byteLength: png.byteLength,
			payloadLength: png.byteLength,
		});
		expect(
			await decodeBinary(new Uint8Array([1, 2, 3, 4]), CELL_DECODE_OPTIONS),
		).toMatchObject({ kind: "binary", reason: "not-text" });
	});

	test("shows the inflated bytes when a gzip payload is not text", async () => {
		const inner = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0, 1, 2, 3]);
		const stored = new Uint8Array(gzipSync(inner));
		const reading = await decodeBinary(stored, DETAIL_DECODE_OPTIONS);
		expect(reading).toMatchObject({
			kind: "binary",
			reason: "not-text",
			compression: "gzip",
			format: "png",
			byteLength: stored.byteLength,
			payloadLength: inner.byteLength,
		});
	});

	test("falls back to the stored bytes when a lookalike header does not inflate", async () => {
		const stored = new Uint8Array([0x1f, 0x8b, 0x08, 0x00, 0x01, 0x02]);
		const reading = await decodeBinary(stored, CELL_DECODE_OPTIONS);
		expect(reading).toMatchObject({ kind: "binary", format: "gzip" });
		expect(reading.compression).toBeUndefined();
	});

	test("refuses payloads over the stored-size budget without decoding", async () => {
		const options: BinaryDecodeOptions = {
			maxBytes: 8,
			maxDecodedBytes: 1024,
		};
		expect(await decodeBinary(utf8('{"a":"long"}'), options)).toMatchObject({
			kind: "binary",
			reason: "too-large",
		});
	});

	test("stops inflating past the decoded-size budget", async () => {
		const stored = gzip("a".repeat(100_000));
		const reading = await decodeBinary(stored, {
			maxBytes: 1024 * 1024,
			maxDecodedBytes: 4096,
		});
		expect(reading).toMatchObject({
			kind: "binary",
			reason: "too-large",
			compression: "gzip",
			format: "gzip",
			payloadLength: stored.byteLength,
		});
	});
});

describe("oversizedBinaryReading", () => {
	test("builds the refusal from the first bytes only", () => {
		const value = Array.from({ length: 5000 }, (_, index) => index % 256);
		const reading = oversizedBinaryReading(value, {
			maxBytes: 1000,
			maxDecodedBytes: 1000,
		});
		expect(reading).toMatchObject({
			kind: "binary",
			reason: "too-large",
			byteLength: 5000,
			payloadLength: 5000,
		});
		if (reading?.kind !== "binary") throw new Error("expected binary");
		expect(reading.head.byteLength).toBe(BINARY_HEAD_BYTES);
	});

	test("leaves payloads within budget to the decoder", () => {
		expect(oversizedBinaryReading([1, 2, 3], CELL_DECODE_OPTIONS)).toBeNull();
	});
});

describe("looksLikeText", () => {
	test("accepts ordinary whitespace and rejects NUL", () => {
		expect(looksLikeText("line one\n\tline two\r\n")).toBe(true);
		expect(looksLikeText("abc\0def")).toBe(false);
	});
});

describe("formatHexDump", () => {
	test("lays out offset, hex and printable ASCII", () => {
		const dump = formatHexDump(utf8("Hello, binary!\n\0!"));
		expect(dump.split("\n")).toEqual([
			"00000000  48 65 6c 6c 6f 2c 20 62 69 6e 61 72 79 21 0a 00  |Hello, binary!..|",
			"00000010  21                                               |!|",
		]);
	});
});

describe("bytesToBase64", () => {
	test("matches the platform encoder", () => {
		const bytes = gzip(JSON.stringify(FEATURE));
		expect(bytesToBase64(bytes)).toBe(Buffer.from(bytes).toString("base64"));
	});
});
