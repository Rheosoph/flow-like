import { describe, expect, test } from "bun:test";
import { stableStringify } from "./stable-stringify";

describe("stableStringify", () => {
	test("key order does not change the output", () => {
		expect(stableStringify({ a: 1, b: 2 })).toBe(
			stableStringify({ b: 2, a: 1 }),
		);
	});

	test("nested key order does not change the output either", () => {
		const left = { outer: { z: 1, a: { n: 2, m: 3 } } };
		const right = { outer: { a: { m: 3, n: 2 }, z: 1 } };
		expect(stableStringify(left)).toBe(stableStringify(right));
	});

	test("array order is preserved, because it is meaningful", () => {
		expect(stableStringify([1, 2])).not.toBe(stableStringify([2, 1]));
	});

	test("objects inside arrays are still normalised", () => {
		expect(stableStringify([{ a: 1, b: 2 }])).toBe(
			stableStringify([{ b: 2, a: 1 }]),
		);
	});

	test("real differences are still detected", () => {
		expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: 2 }));
		expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: "1" }));
		expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: 1, b: 1 }));
	});

	test("handles the shapes an event config actually contains", () => {
		const a = {
			example_messages: ["one", "two"],
			voice: { mode: "disabled", size: "md" },
			history_elements: 5,
			allow_file_upload: true,
		};
		const b = {
			allow_file_upload: true,
			history_elements: 5,
			voice: { size: "md", mode: "disabled" },
			example_messages: ["one", "two"],
		};
		expect(stableStringify(a)).toBe(stableStringify(b));
	});

	test("null and undefined behave like JSON.stringify", () => {
		expect(stableStringify(null)).toBe("null");
		expect(stableStringify({ a: null })).toBe('{"a":null}');
		expect(stableStringify(undefined)).toBeUndefined();
	});
});
