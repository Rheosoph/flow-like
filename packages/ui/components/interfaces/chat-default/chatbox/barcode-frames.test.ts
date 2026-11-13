import { describe, expect, test } from "vitest";
import {
	type DetectedBarcode,
	barcodeMessage,
	fitFrame,
	lumaFromRgba,
	mergeBarcodes,
	trackBarcodes,
	trackedBarcodes,
} from "./barcode-frames";

const code = (format: string, text: string, x = 0): DetectedBarcode => ({
	format,
	text,
	points: [{ x, y: 0 }],
});

describe("barcode frames", () => {
	test("frames shrink to the longest edge and never grow", () => {
		expect(fitFrame(1920, 1080, 960)).toEqual({ width: 960, height: 540 });
		expect(fitFrame(1080, 1920, 960)).toEqual({ width: 540, height: 960 });
		expect(fitFrame(640, 480, 960)).toEqual({ width: 640, height: 480 });
	});

	test("luma weighs green most and ignores alpha", () => {
		const rgba = new Uint8ClampedArray([
			255, 255, 255, 0, 0, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255,
		]);
		expect([...lumaFromRgba(rgba, 4)]).toEqual([255, 0, 76, 149]);
	});

	test("merging keeps one reading per code, the first list winning", () => {
		const merged = mergeBarcodes(
			[code("QR_CODE", "a", 1)],
			[code("QR_CODE", "a", 2), code("EAN_13", "4006381333931")],
		);
		expect(merged).toEqual([
			code("QR_CODE", "a", 1),
			code("EAN_13", "4006381333931"),
		]);
	});

	test("luma is written into a buffer of the frame's size", () => {
		const rgba = new Uint8ClampedArray([255, 255, 255, 255, 0, 0, 0, 255]);
		const buffer = new Uint8Array(2);
		expect(lumaFromRgba(rgba, 2, buffer)).toBe(buffer);
		expect([...buffer]).toEqual([255, 0]);
		expect(lumaFromRgba(rgba, 2, new Uint8Array(5))).toHaveLength(2);
	});

	test("a code stays in view until no frame has read it for the hold time", () => {
		const first = trackBarcodes(new Map(), [code("QR_CODE", "a", 1)], 0, 900);
		const missed = trackBarcodes(first, [], 500, 900);
		expect(trackedBarcodes(missed)).toEqual([code("QR_CODE", "a", 1)]);

		const moved = trackBarcodes(missed, [code("QR_CODE", "a", 7)], 800, 900);
		expect(trackedBarcodes(moved)).toEqual([code("QR_CODE", "a", 7)]);
		expect(trackedBarcodes(trackBarcodes(moved, [], 1701, 900))).toEqual([]);
	});

	test("two equal labels in one frame are both in view", () => {
		const track = trackBarcodes(
			new Map(),
			[
				code("CODE_128", "x", 1),
				code("CODE_128", "x", 2),
				code("QR_CODE", "y"),
			],
			0,
			900,
		);
		expect(trackedBarcodes(track)).toEqual([
			code("CODE_128", "x", 1),
			code("CODE_128", "x", 2),
			code("QR_CODE", "y"),
		]);
	});

	test("codes become a message block, none become nothing", () => {
		expect(
			barcodeMessage(
				[code("EAN_13", "4006381333931"), code("QR_CODE", "https://x.y")],
				"Scanned barcodes:",
			),
		).toBe(
			"Scanned barcodes:\n- EAN_13: 4006381333931\n- QR_CODE: https://x.y",
		);
		expect(barcodeMessage([], "Scanned barcodes:")).toBeUndefined();
	});
});
