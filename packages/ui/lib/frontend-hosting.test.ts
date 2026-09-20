import { describe, expect, test } from "bun:test";
import {
	getHostedFrontendKind,
	getHostedFrontendUrl,
	parseFrontendHosting,
} from "./frontend-hosting";

describe("hosted frontend access configuration", () => {
	test("does not enable hosting for existing or malformed configuration", () => {
		for (const config of [
			undefined,
			null,
			{},
			{ frontend_hosting: true },
			{ frontend_hosting: [] },
			{ frontend_hosting: { enabled: "true", auth_proxy: "false" } },
			{ frontend_hosting: { enabled: true, auth_proxy: "true" } },
			{ frontend_hosting: { enabled: true, allow_anonymous: "true" } },
			{ frontend_hosting: { enabled: true, allow_anonymous: null } },
			{ frontend_hosting: { enabled: true, allow_anonymous: 1 } },
			{ frontend_hosting: { enabled: true, unexpected: true } },
		]) {
			expect(parseFrontendHosting(config)).toEqual({
				enabled: false,
				allow_anonymous: false,
				auth_proxy: true,
			});
		}
	});

	test("enabling hosting requires sign-in until anonymous access is explicitly allowed", () => {
		for (const hosting of [
			{ enabled: true },
			{ enabled: true, auth_proxy: false },
			{ enabled: true, auth_proxy: true },
			{ enabled: true, allow_anonymous: false },
			{ enabled: true, allow_anonymous: false, auth_proxy: false },
			{ enabled: true, allow_anonymous: true, auth_proxy: true },
		]) {
			expect(parseFrontendHosting({ frontend_hosting: hosting })).toEqual({
				enabled: true,
				allow_anonymous: false,
				auth_proxy: true,
			});
		}
	});

	test("allows anonymous access only with both explicit opt-ins", () => {
		for (const hosting of [
			{ enabled: true, allow_anonymous: true },
			{ enabled: true, allow_anonymous: true, auth_proxy: false },
		]) {
			expect(
				parseFrontendHosting({
					frontend_hosting: hosting,
				}),
			).toEqual({ enabled: true, allow_anonymous: true, auth_proxy: false });
		}
		for (const hosting of [
			{ allow_anonymous: true },
			{ enabled: false, allow_anonymous: true, auth_proxy: false },
		]) {
			expect(parseFrontendHosting({ frontend_hosting: hosting })).toEqual({
				enabled: false,
				allow_anonymous: false,
				auth_proxy: true,
			});
		}
	});

	test("routes page targets before considering their trigger type", () => {
		expect(
			getHostedFrontendKind({
				event_type: "simple_chat",
				default_page_id: "custom-page",
			}),
		).toBe("u");
		for (const event_type of ["generic_form", "quick_action"]) {
			expect(getHostedFrontendKind({ event_type })).toBe("f");
		}
		expect(getHostedFrontendKind({ event_type: "simple_chat" })).toBe("c");
		for (const event_type of ["rest", "mcp", "cron", "unknown"]) {
			expect(getHostedFrontendKind({ event_type })).toBeNull();
		}
	});

	test("keeps the alias or ID within one URL path segment", () => {
		expect(
			getHostedFrontendUrl("https://api.example.com/", "c", "my-chat"),
		).toBe("https://api.example.com/c/my-chat");
		expect(
			getHostedFrontendUrl("http://localhost:8080", "f", "event/id?next=a"),
		).toBe("http://localhost:8080/f/event%2Fid%3Fnext%3Da");
	});
});
