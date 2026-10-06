import { describe, expect, test } from "bun:test";
import { displayWith, editHow } from "./completion";

describe("how an edit began (spec M6: the offer's `replaced`)", () => {
	test("an empty field or a fully selected value is replaced", () => {
		expect(editHow("", { start: 0, end: 0 })).toBe("replace");
		expect(editHow("Acme", { start: 0, end: 4 })).toBe("replace");
		expect(editHow("Acme", { start: 0, end: 9 })).toBe("replace");
	});

	test("a caret or a part of the value is edited in place", () => {
		expect(editHow("Acme", { start: 4, end: 4 })).toBe("inPlace");
		expect(editHow("Acme", { start: 0, end: 0 })).toBe("inPlace");
		expect(editHow("Acme", { start: 1, end: 4 })).toBe("inPlace");
	});
});

describe("what the field shows with a suggestion", () => {
	test("the typed text, then the rest of the suggestion", () => {
		expect(displayWith("N", "Nordwind Logistik GmbH")).toBe(
			"Nordwind Logistik GmbH",
		);
		expect(displayWith("n", "Nordwind")).toBe("nordwind");
		expect(displayWith("Nord", null)).toBe("Nord");
		expect(displayWith("", null)).toBe("");
	});
});
