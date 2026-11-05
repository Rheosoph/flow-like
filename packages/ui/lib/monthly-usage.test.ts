import { expect, test } from "bun:test";
import { formatMonthlyLimit, parseMonthlyLimit } from "./monthly-usage";

test("blank inherits, zero is a hard stop, and displayed amounts preserve their units", () => {
	expect(parseMonthlyLimit("  ")).toBeNull();
	expect(parseMonthlyLimit("0")).toBe(0);
	expect(parseMonthlyLimit("0.000001", 1_000_000)).toBe(1);
	expect(parseMonthlyLimit("19.99", 1_000_000)).toBe(19_990_000);
	expect(parseMonthlyLimit("1.5", 60_000)).toBe(90_000);
	expect(parseMonthlyLimit("100")).toBe(100);
});

test("editing a large existing budget preserves every micro-unit", () => {
	for (const scale of [1, 1_000, 1_000_000]) {
		const amount = Number.MAX_SAFE_INTEGER;
		expect(parseMonthlyLimit(formatMonthlyLimit(amount, scale), scale)).toBe(
			amount,
		);
	}
	expect(formatMonthlyLimit(null)).toBe("");
});

test("limits reject negative, fractional calls, sub-micro currency, nonfinite and unsafe values", () => {
	for (const value of [
		"-1",
		"1.2",
		"NaN",
		"Infinity",
		"1e4",
		"9007199254740992",
		"4503599627370495.5",
	]) {
		expect(() => parseMonthlyLimit(value)).toThrow();
	}
	expect(() => parseMonthlyLimit("0.0000001", 1_000_000)).toThrow();
	expect(() => parseMonthlyLimit("9007199255", 1_000_000)).toThrow();
});
