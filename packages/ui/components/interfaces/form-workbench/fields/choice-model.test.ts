import { describe, expect, test } from "bun:test";
import { fitsSegments, jumpTo, targetOf } from "./choice-model";

const plain = { metaKey: false, ctrlKey: false, altKey: false };

describe("which control a choice gets", () => {
	test("2 to 4 short options are segmented, anything else is a select", () => {
		expect(fitsSegments(["EUR", "USD"])).toBe(true);
		expect(fitsSegments(["EUR", "USD", "CHF", "GBP"])).toBe(true);
		expect(fitsSegments(["EUR"])).toBe(false);
		expect(fitsSegments(["a", "b", "c", "d", "e"])).toBe(false);
		expect(fitsSegments(["Chrome", "A rather long option name"])).toBe(false);
		expect(fitsSegments([])).toBe(false);
	});
});

describe("letters and arrows", () => {
	const options = ["EUR", "USD", "CHF", "GBP", "Euro cent"];

	test("a letter jumps to the next option that starts with it, wrapping, ignoring case", () => {
		expect(jumpTo(options, 0, "c")).toBe(2);
		expect(jumpTo(options, 2, "C")).toBe(2);
		expect(jumpTo(options, 2, "e")).toBe(4);
		expect(jumpTo(options, 4, "e")).toBe(0);
		expect(jumpTo(options, 0, "z")).toBe(-1);
	});

	test("← and → wrap; a modifier or a long key name picks nothing", () => {
		expect(targetOf(options, 0, { ...plain, key: "ArrowLeft" })).toBe(4);
		expect(targetOf(options, 4, { ...plain, key: "ArrowRight" })).toBe(0);
		expect(targetOf(options, 0, { ...plain, key: "c" })).toBe(2);
		expect(targetOf(options, 0, { ...plain, metaKey: true, key: "c" })).toBe(
			-1,
		);
		expect(targetOf(options, 0, { ...plain, key: "Enter" })).toBe(-1);
		expect(targetOf(options, 0, { ...plain, key: " " })).toBe(-1);
	});
});
