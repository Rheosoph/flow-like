import { describe, expect, test } from "bun:test";
import { normalizeOptions, toOptionValue } from "./options";

describe("normalizeOptions", () => {
	test("keeps well-formed options untouched", () => {
		expect(
			normalizeOptions([
				{ value: "option1", label: "Option 1" },
				{ value: "option2", label: "Option 2" },
			]),
		).toEqual([
			{ value: "option1", label: "Option 1" },
			{ value: "option2", label: "Option 2" },
		]);
	});

	test("drops empty values, which Radix reserves for the cleared selection", () => {
		expect(
			normalizeOptions([
				{ value: "", label: "Being edited" },
				{ label: "No value at all" },
				{ value: "kept", label: "Kept" },
			]),
		).toEqual([{ value: "kept", label: "Kept" }]);
	});

	test("accepts bare strings and numbers", () => {
		expect(normalizeOptions(["a", 2, "", null])).toEqual([
			{ value: "a", label: "a" },
			{ value: "2", label: "2" },
		]);
	});

	test("falls back to the value when a label is missing", () => {
		expect(normalizeOptions([{ value: 7 }, { value: "x", label: "" }])).toEqual(
			[
				{ value: "7", label: "7" },
				{ value: "x", label: "x" },
			],
		);
	});

	test("keeps the first of repeated values so item keys stay unique", () => {
		expect(
			normalizeOptions([
				{ value: "dup", label: "First" },
				{ value: "dup", label: "Second" },
			]),
		).toEqual([{ value: "dup", label: "First" }]);
	});

	test("tolerates a binding that resolved to something other than a list", () => {
		expect(normalizeOptions(undefined)).toEqual([]);
		expect(normalizeOptions({ value: "a" })).toEqual([]);
	});
});

describe("toOptionValue", () => {
	test("matches the string an option value was normalized to", () => {
		expect(toOptionValue(3)).toBe("3");
		expect(toOptionValue("a")).toBe("a");
	});

	test("renders an unset selection as the empty string", () => {
		expect(toOptionValue(undefined)).toBe("");
		expect(toOptionValue({ nested: true })).toBe("");
	});
});
