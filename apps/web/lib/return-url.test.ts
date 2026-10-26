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
			"/thirdparty/callback?code=connection-code&state=connection-state",
			"/a/app-1/callback",
			"/callback-help",
		]) {
			expect(sanitizeReturnUrl(path)).toBe(path);
		}
		expect(sanitizeReturnUrl("/")).toBeNull();
	});

	it("rejects the login callback as a return destination to prevent replay loops", () => {
		for (const path of [
			"/callback",
			"/callback/",
			"/callback?code=consumed-code&state=old-state",
			"/callback/?code=consumed-code&state=old-state#result",
			"/account/../callback?code=consumed-code",
			"/%2e/callback",
		]) {
			expect(sanitizeReturnUrl(path)).toBeNull();
		}
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
