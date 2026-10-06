import { describe, expect, test } from "bun:test";
import {
	type NavigationRouter,
	buildUseNavigationUrl,
	createNavigate,
	normalizeRoute,
} from "./navigation";

describe("normalizeRoute", () => {
	test("trims and adds the leading slash; empty means the home route", () => {
		expect(normalizeRoute(" support ")).toBe("/support");
		expect(normalizeRoute("/support/chat")).toBe("/support/chat");
		expect(normalizeRoute("   ")).toBe("/");
		expect(normalizeRoute("")).toBe("/");
	});
});

describe("buildUseNavigationUrl", () => {
	test("an empty route is the app's home", () => {
		expect(buildUseNavigationUrl("app1", "")).toBe("/use?id=app1&route=/");
	});

	test("an app route becomes a /use link with the route normalised", () => {
		expect(buildUseNavigationUrl("app1", "support")).toBe(
			"/use?id=app1&route=%2Fsupport",
		);
		expect(buildUseNavigationUrl("app1", "/")).toBe("/use?id=app1&route=%2F");
	});

	test("a query string in the route and explicit parameters both land in the link, explicit ones win", () => {
		const url = buildUseNavigationUrl("app1", "/support?tab=open&x=1", {
			x: "2",
			y: "3",
		});
		const params = new URL(url, "https://app.test").searchParams;
		expect(params.get("id")).toBe("app1");
		expect(params.get("route")).toBe("/support");
		expect(params.get("tab")).toBe("open");
		expect(params.get("x")).toBe("2");
		expect(params.get("y")).toBe("3");
		expect(params.has("eventId")).toBe(false);
	});

	test("a /use link or an absolute URL is kept and only gains parameters", () => {
		expect(buildUseNavigationUrl("app1", "/use?id=other&route=/a")).toBe(
			"/use?id=other&route=/a",
		);
		expect(buildUseNavigationUrl("app1", "/use?id=other", { a: "1" })).toBe(
			"/use?id=other&a=1",
		);
		expect(buildUseNavigationUrl("app1", "/use", { a: "1" })).toBe("/use?a=1");
		expect(buildUseNavigationUrl("app1", "https://x.test/p", { a: "1" })).toBe(
			"https://x.test/p?a=1",
		);
		expect(buildUseNavigationUrl("app1", "https://x.test/p", {})).toBe(
			"https://x.test/p",
		);
	});
});

function recordingRouter() {
	const calls: { readonly kind: "push" | "replace"; readonly href: string }[] =
		[];
	const router: NavigationRouter = {
		push: (href) => calls.push({ kind: "push", href }),
		replace: (href) => calls.push({ kind: "replace", href }),
	};
	return { router, calls };
}

describe("createNavigate", () => {
	test("hands the host's onNavigate the route, replace flag and parameters", () => {
		const { router, calls } = recordingRouter();
		const seen: unknown[][] = [];
		const navigate = createNavigate(
			"app1",
			(route, replace, queryParams) => seen.push([route, replace, queryParams]),
			router,
		);
		navigate({ route: "/support", replace: false });
		navigate({ route: "/chat", replace: true, queryParams: { ticket: "9" } });
		expect(seen).toEqual([
			["/support", false, undefined],
			["/chat", true, { ticket: "9" }],
		]);
		expect(calls).toEqual([]);
	});

	test("without onNavigate the client router gets the /use link", () => {
		const { router, calls } = recordingRouter();
		const navigate = createNavigate("app1", undefined, router);
		navigate({ route: "/support", replace: false });
		navigate({ route: "/chat", replace: true, queryParams: { ticket: "9" } });
		expect(calls).toEqual([
			{ kind: "push", href: "/use?id=app1&route=%2Fsupport" },
			{ kind: "replace", href: "/use?id=app1&route=%2Fchat&ticket=9" },
		]);
	});
});
