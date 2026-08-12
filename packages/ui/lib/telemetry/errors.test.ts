import { describe, expect, it } from "bun:test";
import { isBenignBrowserError } from "./errors";

describe("isBenignBrowserError", () => {
	it("matches the stackless ResizeObserver delivery notices", () => {
		expect(
			isBenignBrowserError(
				"ResizeObserver loop completed with undelivered notifications.",
			),
		).toBe(true);
		expect(isBenignBrowserError("ResizeObserver loop limit exceeded")).toBe(
			true,
		);
	});

	it("matches the notice when it arrives as an Error", () => {
		expect(
			isBenignBrowserError(
				new Error(
					"ResizeObserver loop completed with undelivered notifications",
				),
			),
		).toBe(true);
	});

	it("keeps real failures reportable", () => {
		expect(isBenignBrowserError(new TypeError("x is not a function"))).toBe(
			false,
		);
		expect(isBenignBrowserError("Failed to fetch")).toBe(false);
		expect(isBenignBrowserError(undefined)).toBe(false);
		expect(isBenignBrowserError(null)).toBe(false);
		expect(isBenignBrowserError({ message: 42 })).toBe(false);
	});
});
