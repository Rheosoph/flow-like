import { describe, expect, it } from "bun:test";
import { sanitizeReturnUrl } from "./return-url";

describe("post-login return URLs", () => {
	it("preserves normal local paths, queries and fragments", () => {
		for (const path of [
			"/library",
			"/join?appId=app-1&token=invite%2Ftoken#accept",
			"/c/support?message=hello%20world&sessionId=chat#reply",
			"/f?event=contact&ref=site#form",
			"/u/dashboard?returnTo=https%3A%2F%2Fexample.com",
			"/store?q=%5C%0A%2F%2Fevil.example",
		]) {
			expect(sanitizeReturnUrl(path)).toBe(path);
		}
		expect(sanitizeReturnUrl("/")).toBeNull();
	});

	it("rejects off-origin targets and URL parser whitespace or backslash tricks", () => {
		for (const value of [
			null,
			undefined,
			1,
			{},
			"",
			"library",
			"https://evil.example",
			"javascript:alert(1)",
			"//evil.example/c/support",
			"/\\evil.example/c/support",
			"/c/\\evil.example",
			"/\tevil.example",
			"/c/support\n",
			"/c/support?message=hello world",
			"/c/support#\u0000",
			"/c/support#\u007f",
			"/c/support#\u0085",
			"/c/support#\u00a0",
		]) {
			expect(sanitizeReturnUrl(value)).toBeNull();
		}
	});

	it("rejects paths that become protocol-relative after dot-segment normalization", () => {
		for (const path of [
			"/.//evil.example",
			"/account/..//evil.example/c/support",
			"/%2e//evil.example",
			"/account/%2e%2e//evil.example",
		]) {
			expect(sanitizeReturnUrl(path)).toBeNull();
		}
	});
});
