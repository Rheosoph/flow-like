import { describe, expect, test } from "bun:test";
import {
	normalizeRoutePath,
	routePathsEqual,
	trimTrailingSlashes,
} from "./route-path";

describe("normalizeRoutePath", () => {
	test("preserves long interior slash runs and only trims trailing separators", () => {
		const slashes = "/".repeat(100_000);
		expect(normalizeRoutePath(`/a${slashes}b${slashes}`)).toBe(`/a${slashes}b`);
		expect(normalizeRoutePath(slashes)).toBe("/");
		expect(trimTrailingSlashes(`https://example.com/${slashes}x\n`)).toBe(
			`https://example.com/${slashes}x\n`,
		);
	});

	test("adds the leading slash and drops the trailing one", () => {
		for (const path of [
			"/config",
			"config",
			"config/",
			"/config/",
			"/config//",
		]) {
			expect(normalizeRoutePath(path)).toBe("/config");
		}
	});

	test("treats empty, blank and root paths as the default route", () => {
		for (const path of [undefined, null, "", "   ", "/", "//"]) {
			expect(normalizeRoutePath(path)).toBe("/");
		}
	});

	test("strips a query string or fragment the caller left on the path", () => {
		expect(normalizeRoutePath("/config?config_id=abc")).toBe("/config");
		expect(normalizeRoutePath("/config#section")).toBe("/config");
		expect(normalizeRoutePath("/config/?config_id=abc")).toBe("/config");
	});

	test("keeps nested segments and case", () => {
		expect(normalizeRoutePath("/config/general/")).toBe("/config/general");
		expect(normalizeRoutePath("/Config")).toBe("/Config");
	});

	test("does not fold routes that differ only in case", () => {
		expect(routePathsEqual("/Config", "/config")).toBe(false);
		expect(routePathsEqual("/config/", "config")).toBe(true);
	});
});
