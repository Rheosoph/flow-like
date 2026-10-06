import { describe, expect, test } from "bun:test";
import { cleanAmount, decimalSignOf, nudge } from "./numbers";

const NO_BREAK = String.fromCharCode(0xa0);
const THIN = String.fromCharCode(0x2009);
const NARROW_NO_BREAK = String.fromCharCode(0x202f);

describe("nudge", () => {
	test("↑ in Max pages goes 20 → 21, ⇧↑ adds ten (spec §9 item 32)", () => {
		expect(nudge("20", 1, false, true, "20", null, null)).toBe("21");
		expect(nudge("21", 1, true, true, "20", null, null)).toBe("31");
		expect(nudge("31", -1, true, true, "20", null, null)).toBe("21");
	});

	test("typed decimals are kept: 11769.10 ↑ → 11770.10", () => {
		expect(nudge("11769.10", 1, false, false, "0", null, null)).toBe(
			"11770.10",
		);
		expect(nudge("0.850", -1, false, false, "", null, null)).toBe("0.000");
	});

	test("an empty field starts from its default, else from 0", () => {
		expect(nudge("", 1, false, true, "20", null, null)).toBe("21");
		expect(nudge("", -1, false, true, "20", null, null)).toBe("19");
		expect(nudge("", 1, false, true, "", null, null)).toBe("1");
		expect(nudge("  ", 1, false, true, "", null, null)).toBe("1");
	});

	test("the pin's range clamps, and an empty field starts inside it", () => {
		expect(nudge("10", 1, false, true, "", [5, 10], null)).toBe("10");
		expect(nudge("6", -1, true, true, "", [5, 10], null)).toBe("5");
		expect(nudge("", 1, false, true, "", [5, 10], null)).toBe("6");
		expect(nudge("", 1, false, true, "", [-10, -5], null)).toBe("-5");
	});

	test("without a range a value of 0 or more stops at 0 when the default is 0 or more", () => {
		expect(nudge("0", -1, false, true, "0", null, null)).toBe("0");
		expect(nudge("3", -1, true, true, "", null, null)).toBe("0");
		expect(nudge("0", -1, false, true, "-5", null, null)).toBe("-1");
		expect(nudge("-3", -1, false, true, "", null, null)).toBe("-4");
	});

	test("the pin's step decides the unit, ten steps with Shift", () => {
		expect(nudge("1", 1, false, false, "", null, 0.5)).toBe("1.5");
		expect(nudge("1", 1, true, false, "", null, 0.25)).toBe("3.50");
		expect(nudge("10", 1, false, true, "", null, 5)).toBe("15");
		expect(nudge("10", 1, false, true, "", null, 0)).toBe("11");
	});

	test("a decimal comma reads as a number", () => {
		expect(nudge("1,5", 1, false, false, "", null, null)).toBe("2.5");
	});

	test("text that is not a plain number comes back unchanged", () => {
		expect(nudge("abc", 1, false, true, "20", null, null)).toBe("abc");
		expect(nudge("1e5", 1, false, true, "", null, null)).toBe("1e5");
	});

	test("an unreadable default counts as none", () => {
		expect(nudge("", 1, false, true, "many", null, null)).toBe("1");
	});
});

describe("cleanAmount", () => {
	const decimal = (text: string, sign: "." | ",") =>
		cleanAmount(text, false, sign);
	const whole = (text: string, sign: "." | ",") =>
		cleanAmount(text, true, sign);

	test("both signs: the last one is the decimal sign", () => {
		for (const sign of [".", ","] as const) {
			expect(decimal("€ 11.769,10", sign)).toBe("11769.10");
			expect(decimal("11,769.10", sign)).toBe("11769.10");
			expect(decimal("EUR 11,769.10", sign)).toBe("11769.10");
			expect(decimal("12.345,678", sign)).toBe("12345.678");
			expect(decimal("-1.500,25", sign)).toBe("-1500.25");
		}
	});

	test("spaces, ’ and ' are group signs", () => {
		expect(decimal("11 769,10", ".")).toBe("11769.10");
		expect(decimal(`11${NO_BREAK}769,10`, ".")).toBe("11769.10");
		expect(decimal(`11${THIN}769,10`, ".")).toBe("11769.10");
		expect(decimal(`11${NARROW_NO_BREAK}769,10`, ".")).toBe("11769.10");
		expect(decimal("CHF 1’234.50", ",")).toBe("1234.50");
		expect(decimal("1'234.50", ",")).toBe("1234.50");
	});

	test("one sign several times: group signs", () => {
		expect(decimal("1.234.567", ".")).toBe("1234567");
		expect(decimal("1,234,567", ",")).toBe("1234567");
	});

	test("one sign once with three digits after a 0 is a decimal", () => {
		for (const sign of [".", ","] as const) {
			expect(decimal("0.125", sign)).toBe("0.125");
			expect(decimal("0,850", sign)).toBe("0.850");
			expect(decimal(",500", sign)).toBe("0.500");
		}
	});

	test("one sign once with three digits: a group sign in a whole-number field", () => {
		expect(whole("1.500", ".")).toBe("1500");
		expect(whole("1.500", ",")).toBe("1500");
		expect(whole("1,234", ".")).toBe("1234");
	});

	test("one sign once with three digits in a decimal field: the viewer's sign decides (spec §9 item 35)", () => {
		expect(decimal("1.500", ",")).toBe("1500");
		expect(decimal("1.500", ".")).toBe("1.500");
		expect(decimal("1,234", ".")).toBe("1234");
		expect(decimal("1,234", ",")).toBe("1.234");
	});

	test("one sign once with other than three digits is a decimal", () => {
		expect(decimal("12,5", ".")).toBe("12.5");
		expect(decimal("12.5", ",")).toBe("12.5");
		expect(decimal("3.", ".")).toBe("3");
	});

	test("anything else is pasted as it is (null)", () => {
		expect(decimal("12,34,5", ".")).toBeNull();
		expect(decimal("1.2.3,4.5", ".")).toBeNull();
		expect(whole("1.5", ".")).toBeNull();
		expect(whole("1.234,5", ",")).toBeNull();
		expect(decimal("abc", ".")).toBeNull();
		expect(decimal("", ".")).toBeNull();
		expect(decimal("-", ".")).toBeNull();
		expect(decimal(".", ".")).toBeNull();
	});

	test("plain numbers stay as typed, without leading zeros and without rounding", () => {
		expect(whole("007", ".")).toBe("7");
		expect(whole("0", ".")).toBe("0");
		expect(whole("12345678901234567890", ".")).toBe("12345678901234567890");
		expect(decimal("$42", ".")).toBe("42");
		expect(decimal("42 €", ",")).toBe("42");
		expect(decimal("usd 42", ",")).toBe("42");
	});
});

describe("decimalSignOf", () => {
	test("from Intl in the viewer's language", () => {
		expect(decimalSignOf("en-US")).toBe(".");
		expect(decimalSignOf("en-GB")).toBe(".");
		expect(decimalSignOf("de-DE")).toBe(",");
		expect(decimalSignOf("fr")).toBe(",");
		expect(decimalSignOf("pt-BR")).toBe(",");
		expect(decimalSignOf("ja")).toBe(".");
	});

	test("an unusable tag reads as '.'", () => {
		expect(decimalSignOf("not a locale!")).toBe(".");
		expect(decimalSignOf("")).toBe(".");
	});
});
