import { describe, expect, test } from "bun:test";
import { fitsHalf } from "./fit";

describe("properties that share a row of an object", () => {
	test("only short controls whose label line fits half of the group", () => {
		expect(fitsHalf({ short: true, label: "Net days" }, false)).toBe(true);
		expect(fitsHalf({ short: true, label: "Currency" }, false)).toBe(true);
		expect(fitsHalf({ short: true, label: "Discount percent" }, true)).toBe(
			false,
		);
		expect(fitsHalf({ short: false, label: "Net" }, false)).toBe(false);
	});
});
