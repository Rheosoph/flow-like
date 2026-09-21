import { describe, expect, it } from "bun:test";
import {
	hostedApiPath,
	hostedNavigationPath,
	hostedReturnPath,
	isHostedFrontendPath,
	parseHostedTarget,
} from "./hosted-route";

describe("hosted interface routes", () => {
	it("only bypasses app providers for the bounded hosted route prefixes", () => {
		for (const path of [
			"/c",
			"/f",
			"/u",
			"/c/support",
			"/f/contact/",
			"/u/dashboard",
		])
			expect(isHostedFrontendPath(path)).toBe(true);
		for (const path of [
			null,
			"/",
			"/chat",
			"/callback",
			"/use",
			"/custom",
			"/forms",
			"/users",
		])
			expect(isHostedFrontendPath(path)).toBe(false);
	});
	for (const kind of ["c", "f", "u"] as const) {
		it(`resolves ${kind} event IDs and aliases without static dynamic routes`, () => {
			expect(
				parseHostedTarget(
					new URL(`https://example.com/${kind}/my-alias?sessionId=chat#reply`),
				),
			).toEqual({ kind, slug: "my-alias" });
			expect(
				parseHostedTarget(
					new URL(`https://example.com/${kind}?event=event_123`),
				),
			).toEqual({ kind, slug: "event_123" });
			expect(hostedApiPath({ kind, slug: "event_123" })).toBe(
				`frontend/${kind}/event_123`,
			);
		});
	}
	it("preserves the full hosted return path across sign-in", () => {
		expect(
			hostedReturnPath("/c/help?sessionId=abc&message=hello%20world#reply"),
		).toBe("/c/help?sessionId=abc&message=hello%20world#reply");
		expect(hostedReturnPath("/f?event=feedback&ref=site#form")).toBe(
			"/f?event=feedback&ref=site#form",
		);
	});
	it("rejects off-origin and non-hosted auth return targets", () => {
		for (const value of [
			"https://evil.test/c/alias",
			"//evil.test/c/alias",
			"/\\evil.test/c/alias",
			"/c/a\n",
			"/c/%2f%2fevil.test",
			"/c/%ZZ",
			"/callback",
			"/c/a/extra",
			"/admin",
			"javascript:alert(1)",
		])
			expect(hostedReturnPath(value)).toBeNull();
	});
	it("only navigates to routes returned by the hosting API", () => {
		const routes = [
			{ path: "/contact", event_id: "form_123", kind: "f" as const },
		];
		expect(hostedNavigationPath("/contact?source=chat#details", routes)).toBe(
			"/f/form_123?source=chat#details",
		);
		expect(
			hostedNavigationPath(
				"/use?id=app&route=%2Fcontact",
				routes,
				{ source: "chat" },
				true,
			),
		).toBe("/f?source=chat&event=form_123");
		expect(() => hostedNavigationPath("/private", routes)).toThrow(
			"not been published",
		);
		expect(() => hostedNavigationPath("//evil.test/contact", routes)).toThrow();
		expect(hostedNavigationPath("/contact/", routes)).toBe("/f/form_123");
	});
});
