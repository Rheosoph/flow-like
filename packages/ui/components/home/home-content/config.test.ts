import { describe, expect, it } from "bun:test";
import {
	homeEmbedHref,
	mergeHomeEmbedNavigation,
	parseHomeEmbedTarget,
	safeHomeHref,
} from "./config";

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
