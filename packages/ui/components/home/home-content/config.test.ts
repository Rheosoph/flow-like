import { describe, expect, it } from "bun:test";
import {
	HOME_APP_RENDERINGS,
	homeAppRendering,
	homeEmbedHref,
	homeLinksRendering,
	homeModelRendering,
	mergeHomeEmbedNavigation,
	parseHomeEmbedTarget,
	safeHomeHref,
} from "./config";

describe("home collection rendering", () => {
	it("uses the native Standard cards when no rendering was chosen", () => {
		for (const surface of [undefined, "grid", "card", "borderless", "tinted"]) {
			expect(homeAppRendering({}, surface)).toBe("standard");
			expect(homeModelRendering({}, surface)).toBe("standard");
		}
	});
	it("keeps an explicit card choice independent of the widget surface", () => {
		for (const [rendering] of HOME_APP_RENDERINGS) {
			expect(homeAppRendering({ rendering }, "tinted")).toBe(rendering);
		}
		expect(homeAppRendering({ rendering: "standard" }, "list")).toBe(
			"standard",
		);
		expect(homeModelRendering({ rendering: "standard" }, "list")).toBe(
			"standard",
		);
		expect(homeModelRendering({ rendering: "list" }, "card")).toBe("list");
	});
	it("retains deliberately selected layouts from older saved homes", () => {
		for (const variant of ["list", "icons", "editorial", "carousel"] as const) {
			expect(homeAppRendering({}, variant)).toBe(variant);
		}
		expect(homeAppRendering({}, "spotlight")).toBe("editorial");
		expect(homeModelRendering({}, "list")).toBe("list");
		expect(homeLinksRendering({}, "list")).toBe("list");
		expect(homeLinksRendering({ rendering: "grid" }, "list")).toBe("grid");
		expect(homeLinksRendering({ rendering: "list" }, "tinted")).toBe("list");
	});
	it("falls back to Standard for unsupported or malformed rendering values", () => {
		for (const rendering of ["unknown", {}, 42, "grid"]) {
			expect(homeAppRendering({ rendering }, "list")).toBe("standard");
			expect(homeModelRendering({ rendering }, "list")).toBe("standard");
		}
	});
});

describe("home app embed targets", () => {
	it("keeps query values separate from shell routing and encodes full-view links", () => {
		const target = parseHomeEmbedTarget({
			appId: "app a",
			target: "route",
			route: "/reports?period=week",
			query: "period=month&team=A%26B&id=other&eventId=other&route=evil",
		});
		expect(target).toEqual({
			appId: "app a",
			routePath: "/reports",
			eventId: null,
			queryParams: { period: "month", team: "A&B" },
		});
		expect(homeEmbedHref(target)).toBe(
			"/use?id=app+a&route=%2Freports&period=month&team=A%26B",
		);
	});
	it("preserves question marks inside an inline query value", () => {
		expect(
			parseHomeEmbedTarget({
				appId: "a",
				target: "route",
				route: "/reports?search=what?why&team=A",
			}).queryParams,
		).toEqual({ search: "what?why", team: "A" });
	});
	it("isolates navigation state between widgets and clears an explicit event on route navigation", () => {
		const first = parseHomeEmbedTarget({
			appId: "a",
			target: "event",
			eventId: "chat",
			query: "tab=inbox",
		});
		const second = parseHomeEmbedTarget({
			appId: "a",
			target: "event",
			eventId: "chat",
			query: "tab=inbox",
		});
		const next = mergeHomeEmbedNavigation(first, {
			routePath: "/details",
			eventId: null,
			queryParams: { item: "42" },
		});
		expect(first).toEqual(second);
		expect(next.eventId).toBeNull();
		expect(next.queryParams).toEqual({ item: "42" });
		expect(second.queryParams).toEqual({ tab: "inbox" });
	});
	it("ignores stale route and event selections in landing mode", () => {
		expect(
			parseHomeEmbedTarget({
				appId: "a",
				target: "landing",
				route: "/old",
				eventId: "old",
			}),
		).toEqual({ appId: "a", routePath: "/", eventId: null, queryParams: {} });
	});
	it("accepts usable links and rejects executable or protocol-relative links", () => {
		expect(safeHomeHref("/use?id=a")).toBe("/use?id=a");
		expect(safeHomeHref("https://example.org")).toBe("https://example.org/");
		expect(safeHomeHref("javascript:alert(1)")).toBeUndefined();
		expect(safeHomeHref("//example.org")).toBeUndefined();
		expect(safeHomeHref("/\\example.org")).toBeUndefined();
		expect(safeHomeHref("java\nscript:alert(1)")).toBeUndefined();
	});
});
