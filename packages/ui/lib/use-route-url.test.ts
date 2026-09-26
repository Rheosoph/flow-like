import { describe, expect, test } from "bun:test";
import { appRouteUrl, parseAppRouteTarget } from "./app-route-url";
import {
	isUsePathname,
	pathUseUrl,
	queryUseUrl,
	readUseRoutePath,
} from "./use-route-url";

const at = (href: string) => new URL(href, "https://app.test");

describe("native app route queries", () => {
	test("keeps exported shell links and unrelated paths unchanged", () => {
		for (const href of [
			"/use?id=app&route=%2Forders&value=a%20b#details",
			"/use?id=app&eventId=event",
			"/users?route=%2Forders",
		])
			expect(queryUseUrl(at(href))).toBe(href);
	});

	test("converts deep links without losing shell or app query data", () => {
		const url = at(
			"/use/orders/123?id=app&route=%2Fold&route=%2Fstale&eventId=event&appQuery=id%3Dorder%26route%3D%252Fapp-owned&tag=a&tag=b#details",
		);
		const before = url.href;
		const next = at(queryUseUrl(url));
		expect(url.href).toBe(before);
		expect(next.pathname).toBe("/use");
		expect(next.searchParams.getAll("route")).toEqual(["/orders/123"]);
		expect([...next.searchParams].filter(([key]) => key !== "route")).toEqual(
			[...url.searchParams].filter(([key]) => key !== "route"),
		);
		expect(next.hash).toBe("#details");
	});

	test("preserves explicit root routes and decodes path segments once", () => {
		for (const route of ["/", "/café sale/東京/50%", "/literal%2Fsegment"]) {
			const url = at("/use?id=app&eventId=event");
			url.searchParams.set("route", route);
			const next = at(queryUseUrl(at(pathUseUrl(url))));
			expect(next.pathname).toBe("/use");
			expect(next.searchParams.get("route")).toBe(route);
		}
		expect(() => queryUseUrl(at("/use/a%2Fb?id=app"))).toThrow();
	});
});

describe("web app route paths", () => {
	test("recognizes only the use shell and its descendants", () => {
		for (const path of ["/use", "/use/", "/use/orders"])
			expect(isUsePathname(path)).toBe(true);
		for (const path of [null, "", "/users", "/useful", "/Use", "/c/orders"])
			expect(isUsePathname(path)).toBe(false);
	});

	test("distinguishes an implicit Event target from an explicit root route", () => {
		expect(readUseRoutePath("/use")).toBeUndefined();
		expect(readUseRoutePath("/users")).toBeUndefined();
		expect(readUseRoutePath("/use/")).toBe("/");
		expect(pathUseUrl(at("/use?id=app&eventId=event"))).toBe(
			"/use?id=app&eventId=event",
		);
		expect(pathUseUrl(at("/use?id=app&route=%2F&eventId=event"))).toBe(
			"/use/?id=app&eventId=event",
		);
		expect(pathUseUrl(at("/use?id=app&route="))).toBe("/use/?id=app");
	});

	test("moves the route while preserving nested app query and shell state", () => {
		const url = at(
			appRouteUrl(
				"chosen app",
				parseAppRouteTarget("/orders/123?id=order&tag=one&tag=two", [
					{ name: "route", value: "/app-owned" },
					{ name: "raw", value: "café + 50% & # ? = %2B" },
				]),
			),
		);
		url.searchParams.append("sessionId", "session-one");
		url.searchParams.append("sessionId", "session-two");
		url.searchParams.set("eventId", "event");
		url.searchParams.set("message", "hello + world");
		url.hash = "#details";
		const before = url.href;
		const next = at(pathUseUrl(url));
		expect(url.href).toBe(before);
		expect(next.pathname).toBe("/use/orders/123");
		expect(next.searchParams.has("route")).toBe(false);
		expect([...next.searchParams]).toEqual(
			[...url.searchParams].filter(([key]) => key !== "route"),
		);
		expect(next.hash).toBe("#details");
	});

	test("the path wins over stale legacy route parameters", () => {
		const url = at(
			"/use/orders?id=app&route=%2Fold&route=%2Fother&tag=a&tag=b",
		);
		expect(pathUseUrl(url)).toBe("/use/orders?id=app&tag=a&tag=b");
		expect(pathUseUrl(at("/use/?id=app&route=%2Fold"))).toBe("/use/?id=app");
	});

	test("encodes authored route segments once and decodes the transport once", () => {
		for (const path of [
			"/café sale/東京/50%",
			"/encoded%20path",
			"/reports/12:30",
			"/literal%2Fsegment",
			"/literal%2e%2e",
			"/use",
			"/users",
		]) {
			const url = at("/use?id=app");
			url.searchParams.set("route", path);
			const next = at(pathUseUrl(url));
			expect(readUseRoutePath(next.pathname)).toBe(path);
			expect(pathUseUrl(next)).toBe(`${next.pathname}${next.search}`);
		}
		expect(readUseRoutePath("/use/encoded%2520path")).toBe("/encoded%20path");
		expect(readUseRoutePath("/use/encoded%20path")).toBe("/encoded path");
		expect(readUseRoutePath("/use/Reports/Quarter/")).toBe("/Reports/Quarter");
	});

	test("rejects invalid deep paths before route normalization", () => {
		for (const pathname of [
			"/use/../flow",
			"/use/a/./b",
			"/use/%2e%2e/flow",
			"/use/%2Foutside",
			"/use/a%2Fb",
			"/use//outside",
			"/use/%5Coutside",
			"/use/a\\b",
			"/use/%00",
			"/use/%0A",
			"/use/%23fragment",
			"/use/%3Fquery",
			"/use/%invalid",
			"/use/%E0%A4",
		])
			expect(() => readUseRoutePath(pathname)).toThrow();
		expect(() => pathUseUrl(at("/use/a%2Fb?id=app&route=/safe"))).toThrow();
	});

	test("leaves invalid legacy paths unchanged without normalizing traversal", () => {
		for (const route of [
			"/../flow",
			"/a/./b",
			"https://elsewhere.test",
			"//elsewhere.test",
			"/a\\b",
			"/a\n",
			"/path#fragment",
			"/path?query=value",
		]) {
			const url = at("/use?id=app&eventId=event#section");
			url.searchParams.set("route", route);
			expect(pathUseUrl(url)).toBe(`${url.pathname}${url.search}${url.hash}`);
		}
	});

	test("does not rewrite unrelated URLs or query data without a route", () => {
		for (const href of [
			"/users?id=app&route=%2Forders#hash",
			"/c/support?route=%2Forders",
			"/use?id=app&appQuery=route%3D%252Forders",
			"/use/orders?id=app&value=a%20b#hash",
		])
			expect(pathUseUrl(at(href))).toBe(href);
	});
});
