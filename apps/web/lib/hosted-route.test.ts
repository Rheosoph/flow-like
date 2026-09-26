import { describe, expect, it } from "bun:test";
import {
	hostedApiPath,
	hostedNavigationPath,
	hostedReturnPath,
	isHostedFrontendPath,
	isHostedQueryPath,
	parseHostedTarget,
} from "./hosted-route";

const parse = (path: string) =>
	parseHostedTarget(new URL(path, "https://example.com"));

describe("hosted interface routes", () => {
	it("only bypasses app providers for the app-scoped hosted prefix", () => {
		for (const path of ["/a", "/a/", "/a/app-1", "/a/app-1/support/"])
			expect(isHostedFrontendPath(path)).toBe(true);
		for (const path of [
			null,
			"/",
			"/about",
			"/account",
			"/admin",
			"/ai",
			"/app",
			"/c/support",
			"/f/contact",
			"/u/dashboard",
			"/callback",
			"/use",
		])
			expect(isHostedFrontendPath(path)).toBe(false);
		expect(isHostedQueryPath("/a")).toBe(true);
		expect(isHostedQueryPath("/a/")).toBe(true);
		expect(isHostedQueryPath("/a/app-1")).toBe(false);
	});

	it("reads the app and route from the path form", () => {
		expect(parse("/a/app-1")).toEqual({ app: "app-1", route: "/" });
		expect(parse("/a/app-1/")).toEqual({ app: "app-1", route: "/" });
		expect(parse("/a/app-1/support?sessionId=chat#reply")).toEqual({
			app: "app-1",
			route: "/support",
		});
		expect(parse("/a/app_1/support/demo//")).toEqual({
			app: "app_1",
			route: "/support/demo",
		});
		expect(parse("/a/app-1/my%20page/%C3%BCber")).toEqual({
			app: "app-1",
			route: "/my page/über",
		});
		expect(parse("/a/app-1/Config")).toEqual({
			app: "app-1",
			route: "/Config",
		});
		expect(parse("/a/%61pp-1/x")).toEqual({ app: "app-1", route: "/x" });
	});

	it("reads the app and route from the query form", () => {
		expect(parse("/a?app=app-1&route=%2Fcontact&ref=site")).toEqual({
			app: "app-1",
			route: "/contact",
		});
		expect(parse("/a/?app=app-1&route=contact/")).toEqual({
			app: "app-1",
			route: "/contact",
		});
		expect(parse("/a?app=app-1")).toEqual({ app: "app-1", route: "/" });
		expect(parse("/a?app=app-1&route=")).toEqual({ app: "app-1", route: "/" });
	});

	it("keeps the served variant", () => {
		expect(parse("/a/app-1/support?__variant=beta")).toEqual({
			app: "app-1",
			route: "/support",
			variant: "beta",
		});
		expect(parse("/a?app=app-1&route=%2F&__variant=beta")).toEqual({
			app: "app-1",
			route: "/",
			variant: "beta",
		});
	});

	it("rejects malformed app ids and routes", () => {
		for (const path of [
			"/a",
			"/a?route=%2Fcontact",
			"/a//support",
			"/a/app.1",
			"/a/app%201",
			`/a/${"x".repeat(129)}`,
			"/a/%2f%2fevil.test",
			"/a/%ZZ",
			"/a/app-1/%ZZ",
			"/a/app-1/a%2Fb",
			"/a/app-1/a%5Cb",
			"/a/app-1/a%3Fb",
			"/a/app-1/a%23b",
			"/a/app-1/a%00b",
			"/a?app=app-1&route=%2Fa%5Cb",
			"/a?app=bad%20app&route=%2F",
			"/c/support",
			"/use?id=app-1&route=%2Fsupport",
		])
			expect(parse(path)).toBeNull();
	});

	it("addresses the runtime API by app", () => {
		expect(hostedApiPath({ app: "app-1", route: "/support" })).toBe(
			"frontend/a/app-1",
		);
	});

	it("preserves the full hosted return path across sign-in", () => {
		expect(
			hostedReturnPath(
				"/a/app-1/help?sessionId=abc&message=hello%20world#reply",
			),
		).toBe("/a/app-1/help?sessionId=abc&message=hello%20world#reply");
		expect(hostedReturnPath("/a/app-1")).toBe("/a/app-1");
		expect(
			hostedReturnPath("/a?app=app-1&route=%2Ffeedback&ref=site#form"),
		).toBe("/a?app=app-1&route=%2Ffeedback&ref=site#form");
	});

	it("rejects off-origin and non-hosted auth return targets", () => {
		for (const value of [
			"https://evil.test/a/app-1",
			"//evil.test/a/app-1",
			"/\\evil.test/a/app-1",
			"/a/app-1\n",
			"/a/%2f%2fevil.test",
			"/a/app-1/%2f%2fevil.test",
			"/a/%ZZ",
			"/a",
			"/c/help",
			"/callback",
			"/admin",
			"javascript:alert(1)",
		])
			expect(hostedReturnPath(value)).toBeNull();
	});

	it("navigates within the same app to routes returned by the hosting API", () => {
		const routes = [
			{ path: "/", event_id: "chat_123", kind: "c" as const },
			{ path: "/contact", event_id: "form_123", kind: "f" as const },
			{ path: "/my page", event_id: "page_123", kind: "u" as const },
		];
		expect(
			hostedNavigationPath("app-1", "/contact?source=chat#details", routes),
		).toBe("/a/app-1/contact?source=chat#details");
		expect(hostedNavigationPath("app-1", "/contact/", routes)).toBe(
			"/a/app-1/contact",
		);
		expect(hostedNavigationPath("app-1", "contact", routes)).toBe(
			"/a/app-1/contact",
		);
		expect(hostedNavigationPath("app-1", "/", routes)).toBe("/a/app-1");
		expect(hostedNavigationPath("app-1", "/my%20page", routes)).toBe(
			"/a/app-1/my%20page",
		);
		expect(
			hostedNavigationPath(
				"app-1",
				"/use?id=app-1&route=%2Fcontact&eventId=x&sessionId=s&app=y&keep=1",
				routes,
				{ source: "chat" },
			),
		).toBe("/a/app-1/contact?keep=1&source=chat");
		expect(hostedNavigationPath("app-1", "/use/contact?id=app-1", routes)).toBe(
			"/a/app-1/contact",
		);
	});

	it("keeps query routing on the /a entry point", () => {
		const routes = [{ path: "/contact" }];
		expect(
			hostedNavigationPath(
				"app-1",
				"/use?id=app-1&route=%2Fcontact",
				routes,
				{ source: "chat", route: "/other", app: "other" },
				true,
			),
		).toBe("/a?app=app-1&route=%2Fcontact&source=chat");
		expect(
			parse(hostedNavigationPath("app-1", "/contact#form", routes, {}, true)),
		).toEqual({ app: "app-1", route: "/contact" });
	});

	it("refuses routes that are not published on this link", () => {
		const routes = [{ path: "/contact" }];
		expect(() => hostedNavigationPath("app-1", "/private", routes)).toThrow(
			"The page /private has not been published on this link.",
		);
		expect(() => hostedNavigationPath("app-1", "/Contact", routes)).toThrow(
			"not been published",
		);
		expect(() =>
			hostedNavigationPath("app-1", "//evil.test/contact", routes),
		).toThrow();
		expect(() =>
			hostedNavigationPath("app-1", "https://evil.test/contact", routes),
		).toThrow();
		expect(() => hostedNavigationPath("app-1", "/contact%3Fx", routes)).toThrow(
			"does not point to a published app route",
		);
		expect(() => hostedNavigationPath("app-1", "/%ZZ", routes)).toThrow(
			"does not point to a published app route",
		);
	});
});
