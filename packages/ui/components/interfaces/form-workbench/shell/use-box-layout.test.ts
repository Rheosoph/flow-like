import { describe, expect, test } from "bun:test";
import { layoutFor } from "../testing/layouts";
import { layoutOf, sameLayout } from "./use-box-layout";

const FINE = { touch: false, finePointer: true };
const COARSE = { touch: true, finePointer: false };

describe("layoutOf", () => {
	test("split starts at 900 px, compare when the stage beside the 400 px rail is 600 px", () => {
		expect(layoutOf({ width: 899, height: 700 }, FINE)).toMatchObject({
			split: false,
			compare: false,
		});
		expect(layoutOf({ width: 900, height: 700 }, FINE)).toMatchObject({
			split: true,
			compare: false,
		});
		expect(layoutOf({ width: 999, height: 700 }, FINE).compare).toBe(false);
		expect(layoutOf({ width: 1000, height: 700 }, FINE)).toMatchObject({
			split: true,
			compare: true,
		});
	});

	test("the pointer and the box are separate flags", () => {
		expect(layoutOf({ width: 820, height: 900 }, FINE)).toMatchObject({
			split: false,
			touch: false,
			finePointer: true,
		});
		expect(layoutOf({ width: 1024, height: 768 }, COARSE)).toMatchObject({
			split: true,
			touch: true,
			finePointer: false,
		});
	});

	test("fractions never round a box above a threshold", () => {
		expect(layoutOf({ width: 899.9, height: 600.7 }, FINE)).toMatchObject({
			width: 899,
			height: 600,
			split: false,
		});
	});

	test("agrees with the layout the fixtures and the harness hand out", () => {
		for (const [width, height] of [
			[360, 640],
			[390, 728],
			[899, 700],
			[900, 700],
			[1000, 836],
			[1360, 836],
		] as const) {
			expect(layoutOf({ width, height }, FINE)).toEqual(
				layoutFor(width, height),
			);
			expect(layoutOf({ width, height }, COARSE)).toEqual(
				layoutFor(width, height, { touch: true }),
			);
		}
	});
});

describe("sameLayout", () => {
	const base = layoutOf({ width: 1360, height: 836 }, FINE);

	test("compares every field", () => {
		expect(sameLayout(base, { ...base })).toBe(true);
		for (const change of [
			{ width: 1359 },
			{ height: 800 },
			{ split: false },
			{ touch: true },
			{ finePointer: false },
			{ compare: false },
		]) {
			expect(sameLayout(base, { ...base, ...change })).toBe(false);
		}
	});

	test("null only equals null", () => {
		expect(sameLayout(null, null)).toBe(true);
		expect(sameLayout(null, base)).toBe(false);
		expect(sameLayout(base, undefined)).toBe(false);
	});
});
