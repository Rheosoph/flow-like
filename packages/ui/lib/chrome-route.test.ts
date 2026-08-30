import { describe, expect, it } from "bun:test";
import { ownsWindowChrome } from "./chrome-route";

describe("ownsWindowChrome", () => {
	it("claims the board editor", () => {
		expect(ownsWindowChrome("/flow")).toBe(true);
	});

	it("ignores the query string the board is always mounted with", () => {
		expect(ownsWindowChrome("/flow?id=board&app=app")).toBe(true);
	});

	it("tolerates a trailing slash", () => {
		expect(ownsWindowChrome("/flow/")).toBe(true);
	});

	it("does not claim routes that merely start with the same characters", () => {
		expect(ownsWindowChrome("/flow-templates")).toBe(false);
		expect(ownsWindowChrome("/flowpilot")).toBe(false);
	});

	it("does not claim nested routes", () => {
		expect(ownsWindowChrome("/flow/settings")).toBe(false);
	});

	it("keeps the chrome for everything else", () => {
		expect(ownsWindowChrome("/")).toBe(false);
		expect(ownsWindowChrome("/library/config/flows")).toBe(false);
		expect(ownsWindowChrome(undefined)).toBe(false);
		expect(ownsWindowChrome(null)).toBe(false);
		expect(ownsWindowChrome("")).toBe(false);
	});
});
