import { describe, expect, test } from "bun:test";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import {
	appRouteHref,
	initialServiceEvent,
	resolveServiceNavigation,
	serviceEventHref,
} from "./navigation";

const events = [
	{ id: "home", route: "/", is_default: true, default_page_id: "p1" },
	{ id: "reports", route: "/reports/", default_page_id: "p2" },
	{ id: "chat", route: "/assistant", event_type: "simple_chat" },
] as unknown as IEvent[];
const current = new URL("https://device.example:8443/ui/?sessionId=s1");
const resolve = (href: string) =>
	resolveServiceNavigation(href, "project", events, current);

describe("standalone service navigation", () => {
	test("maps app links and navigate messages onto the exported shell", () => {
		expect(resolve("/use?id=project&route=/reports&filter=open")).toEqual({
			kind: "event",
			eventId: "reports",
			href: "/ui/?route=%2Freports&filter=open",
		});
		expect(resolve("/use/assistant")).toEqual({
			kind: "event",
			eventId: "chat",
			href: "/ui/?route=%2Fassistant",
		});
		expect(resolve("/use?id=project")).toMatchObject({
			kind: "event",
			eventId: "home",
		});
		expect(resolve(appRouteHref("/reports?tab=1", { year: "2026" }))).toEqual({
			kind: "event",
			eventId: "reports",
			href: "/ui/?tab=1&year=2026&route=%2Freports",
		});
		const shared = resolve("/use?route=/reports");
		if (shared.kind !== "event") throw new Error("expected an event route");
		expect(resolve(shared.href)).toEqual(shared);
	});

	test("keeps query updates on the shell and refuses what the service cannot show", () => {
		expect(resolve("/ui/?sessionId=s2")).toEqual({
			kind: "query",
			href: "/ui/?sessionId=s2",
		});
		expect(resolve("/use?id=other&route=/reports")).toEqual({
			kind: "unsupported",
		});
		expect(resolve("/use?route=/missing")).toEqual({ kind: "unsupported" });
		expect(resolve("/use/%2F..")).toEqual({ kind: "unsupported" });
		expect(resolve("/library/config?id=project")).toEqual({
			kind: "unsupported",
		});
		expect(resolve("https://example.com/docs")).toEqual({
			kind: "external",
			href: "https://example.com/docs",
		});
		expect(resolve("javascript:alert(1)")).toEqual({ kind: "unsupported" });
	});

	test("a query update on a routed Page keeps that route in the URL", () => {
		const onReports = new URL(
			"https://device.example:8443/ui/?route=%2Freports",
		);
		const update = resolveServiceNavigation(
			"?tab=2",
			"project",
			events,
			onReports,
		);
		expect(update).toEqual({
			kind: "query",
			href: "/ui/?tab=2&route=%2Freports",
		});
		if (update.kind !== "query") throw new Error("expected a query update");
		expect(
			initialServiceEvent(new URL(update.href, onReports).search, events)?.id,
		).toBe("reports");
		expect(
			resolveServiceNavigation(
				"?route=%2Fassistant",
				"project",
				events,
				onReports,
			),
		).toMatchObject({ kind: "event", eventId: "chat" });
	});

	test("chat navigation keeps app links and external URLs as targets", () => {
		expect(
			resolve(appRouteHref("/use?id=project&route=/reports", { year: "2026" })),
		).toEqual({
			kind: "event",
			eventId: "reports",
			href: "/ui/?route=%2Freports&year=2026",
		});
		expect(resolve(appRouteHref("/use/assistant"))).toEqual({
			kind: "event",
			eventId: "chat",
			href: "/ui/?route=%2Fassistant",
		});
		expect(resolve(appRouteHref("https://docs.example/guide#setup"))).toEqual({
			kind: "external",
			href: "https://docs.example/guide#setup",
		});
		expect(appRouteHref("https://docs.example/guide", { q: "maps" })).toBe(
			"https://docs.example/guide?q=maps",
		);
		expect(appRouteHref("/user/settings")).toBe(
			"/use?route=%2Fuser%2Fsettings",
		);
	});

	test("a reloaded or shared URL reopens its route", () => {
		expect(initialServiceEvent("?route=%2Freports", events)?.id).toBe(
			"reports",
		);
		expect(initialServiceEvent("?route=%2Fmissing", events)?.id).toBe("home");
		expect(initialServiceEvent("", events)?.id).toBe("home");
		for (const event of events)
			expect(
				initialServiceEvent(
					new URL(serviceEventHref(event), current).search,
					events,
				)?.id,
			).toBe(event.id);
		expect(
			serviceEventHref({ id: "untitled", route: "" } as unknown as IEvent),
		).toBe("/ui/");
	});
});
