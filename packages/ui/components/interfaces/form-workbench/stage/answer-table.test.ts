import { describe, expect, test } from "bun:test";
import { isNumericText, numericColumnsOf } from "./answer-table";

describe("isNumericText", () => {
	test("numbers, amounts and percentages as answers write them", () => {
		for (const text of [
			"12",
			"€ 4,920.00",
			"€ 410.00",
			"1.234,50 €",
			"-3.5 %",
			"CHF 1’200",
			"USD 12,000",
			"−7",
			" 0,85 ",
		])
			expect(isNumericText(text)).toBe(true);
	});

	test("an amount tied with no-break spaces is still a number", () => {
		for (const text of ["€\u{a0}410.00", "19\u{a0}%", "1.234,50\u{a0}€"])
			expect(isNumericText(text)).toBe(true);
	});

	test("codes, dates, words and mixed text are not numbers", () => {
		for (const text of [
			"",
			"RE-2026-0917",
			"2026-09-17",
			"12:30",
			"Pallet transport",
			"€",
			"12 pallets",
			"1e5",
		])
			expect(isNumericText(text)).toBe(false);
	});
});

describe("numericColumnsOf", () => {
	const head = ["#", "Description", "Qty", "Unit price", "Net"];
	const rows = [
		["1", "Pallet transport Hamburg–Munich", "12", "€ 410.00", "€ 4,920.00"],
		["2", "Cold-chain surcharge", "12", "€ 85.00", "€ 1,020.00"],
	];

	test("the canvas table: Qty, Unit price and Net; the running index stays a label", () => {
		expect(numericColumnsOf(head, rows)).toEqual([
			false,
			false,
			true,
			true,
			true,
		]);
	});

	test("empty cells keep a column numeric; an empty column or one word does not", () => {
		expect(
			numericColumnsOf(
				["Amount", "Note", "Total"],
				[
					["12", "", "€ 4.00"],
					["", "", "n/a"],
				],
			),
		).toEqual([true, false, false]);
	});

	test("rows wider than the header are measured too", () => {
		expect(numericColumnsOf([], [["a", "3"]])).toEqual([false, true]);
	});
});
