import { describe, expect, test } from "bun:test";
import {
	ellipsizeLabel,
	fitNivoLegendItemWidth,
	shiftIntoRange,
} from "./nivo-chart-fit";

const measure = (text: string) => Array.from(text).length * 6;

describe("fitNivoLegendItemWidth", () => {
	test("fits the longest label plus symbol and gap", () => {
		expect(
			fitNivoLegendItemWidth(
				["Ausgaben · Minor Units", "Einnahmen · Minor Units"],
				0,
				measure,
			),
		).toBe(23 * 6 + 1 + 20 + 16);
	});

	test("caps one long label instead of growing every item", () => {
		expect(fitNivoLegendItemWidth(["x".repeat(80), "y"], 0, measure)).toBe(180);
	});

	test("shares a row's width between its items", () => {
		expect(
			fitNivoLegendItemWidth(["Revenue", "Cost", "Margin"], 300, measure),
		).toBe(Math.min(7 * 6 + 37, 100));
		expect(fitNivoLegendItemWidth(["a".repeat(30), "b"], 200, measure)).toBe(
			100,
		);
	});

	test("keeps a floor when a row is crowded", () => {
		expect(
			fitNivoLegendItemWidth(
				Array.from({ length: 20 }, (_, index) => `Series ${index}`),
				400,
				measure,
			),
		).toBe(48);
	});

	test("uses the floor for short labels", () => {
		expect(fitNivoLegendItemWidth(["A"], 0, measure)).toBe(48);
	});
});

describe("ellipsizeLabel", () => {
	test("returns labels that fit unchanged", () => {
		expect(ellipsizeLabel("Revenue", 42, measure)).toBe("Revenue");
	});

	test("cuts to the longest prefix that fits with an ellipsis", () => {
		expect(ellipsizeLabel("Ausgaben · Minor Units", 60, measure)).toBe(
			"Ausgaben…",
		);
	});

	test("drops whitespace before the ellipsis", () => {
		expect(ellipsizeLabel("Net revenue", 30, measure)).toBe("Net…");
	});

	test("never splits a surrogate pair", () => {
		expect(ellipsizeLabel("📈📈📈📈", 18, measure)).toBe("📈📈…");
	});

	test("falls back to a bare ellipsis", () => {
		expect(ellipsizeLabel("Revenue", 4, measure)).toBe("…");
	});
});

describe("shiftIntoRange", () => {
	test("leaves a range that already fits", () => {
		expect(shiftIntoRange(10, 50, 0, 100)).toBe(0);
	});

	test("pushes past the start edge back in", () => {
		expect(shiftIntoRange(-20, 30, 0, 100)).toBe(20);
	});

	test("pulls past the end edge back in", () => {
		expect(shiftIntoRange(80, 130, 0, 100)).toBe(-30);
	});

	test("start-aligns a range larger than the bounds", () => {
		expect(shiftIntoRange(30, 180, 0, 100)).toBe(-30);
	});
});
