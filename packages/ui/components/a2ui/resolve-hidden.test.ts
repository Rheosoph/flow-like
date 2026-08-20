import { describe, expect, it } from "bun:test";
import { resolveHidden } from "./resolve-hidden";
import type { BoundValue } from "./types";

const literalResolve = (bound: BoundValue, fallback?: unknown): unknown => {
	if (bound === null || bound === undefined) return fallback ?? bound;
	if (typeof bound !== "object") return bound;
	if ("literalString" in bound) return bound.literalString;
	if ("literalBool" in bound) return bound.literalBool;
	if ("path" in bound) return bound.defaultValue ?? fallback;
	return fallback;
};

describe("resolveHidden", () => {
	it("treats an absent property as visible", () => {
		expect(resolveHidden(undefined, literalResolve)).toBe(false);
	});

	it("honours a boolean literal", () => {
		expect(resolveHidden({ literalBool: true }, literalResolve)).toBe(true);
		expect(resolveHidden({ literalBool: false }, literalResolve)).toBe(false);
	});

	it("accepts the string form a text-typed pin produces", () => {
		expect(resolveHidden({ literalString: "true" }, literalResolve)).toBe(true);
		expect(resolveHidden({ literalString: "false" }, literalResolve)).toBe(
			false,
		);
	});

	it("does not treat an arbitrary non-empty string as hidden", () => {
		expect(resolveHidden({ literalString: "yes" }, literalResolve)).toBe(false);
	});

	it("falls back to the binding default when the path is unresolved", () => {
		expect(
			resolveHidden({ path: "/inputs/hidden", defaultValue: true }, (b, f) =>
				literalResolve(b as BoundValue, f),
			),
		).toBe(true);
	});
});
