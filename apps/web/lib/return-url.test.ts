import { describe, expect, it } from "bun:test";
import { sanitizeReturnUrl } from "./return-url";

describe("post-login return URLs", () => {
	it("preserves normal local paths, queries and fragments", () => {
		for (const path of [
			"/library",
			"/join?appId=app-1&token=invite%2Ftoken#accept",
			"/a/app-1/support?message=hello%20world&sessionId=chat#reply",
			"/a?app=app-1&route=%2Fcontact&ref=site#form",
			"/a/app-1/dashboard?returnTo=https%3A%2F%2Fexample.com",
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
			"//evil.example/a/app-1/support",
			"/\\evil.example/a/app-1/support",
			"/a/app-1/\\evil.example",
			"/\tevil.example",
			"/a/app-1/support\n",
			"/a/app-1/support?message=hello world",
			"/a/app-1/support#\u0000",
			"/a/app-1/support#\u007f",
			"/a/app-1/support#\u0085",
			"/a/app-1/support#\u00a0",
		]) {
			expect(sanitizeReturnUrl(value)).toBeNull();
		}
	});

	it("rejects paths that become protocol-relative after dot-segment normalization", () => {
		for (const path of [
			"/.//evil.example",
			"/account/..//evil.example/a/app-1/support",
			"/%2e//evil.example",
			"/account/%2e%2e//evil.example",
		]) {
			expect(sanitizeReturnUrl(path)).toBeNull();
		}
	});
});
