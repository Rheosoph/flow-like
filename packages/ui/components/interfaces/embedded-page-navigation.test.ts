import { describe, expect, test } from "bun:test";
import {
	isSafeEmbeddedExternalHref,
	resolveEmbeddedPageNavigation,
} from "./embedded-page-navigation";

describe("resolveEmbeddedPageNavigation", () => {
	test("keeps an app route and its query inside the embedded target", () => {
		expect(
			resolveEmbeddedPageNavigation(
				{
					type: "navigateTo",
					route: "/orders?status=open&sort=old",
					replace: false,
					queryParams: { sort: "new" },
				},
				"orders-app",
				{ stale: "value" },
			),
		).toEqual({
			routePath: "/orders",
			eventId: null,
			queryParams: { status: "open", sort: "new" },
		});
	});

	test("extracts same-app /use targets without leaking shell parameters", () => {
		expect(
			resolveEmbeddedPageNavigation(
				{
					type: "navigateTo",
					route: "/use?id=orders-app&eventId=details-page&order=42",
					replace: true,
				},
				"orders-app",
				{},
			),
		).toEqual({
			routePath: "/",
			eventId: "details-page",
			queryParams: { order: "42" },
		});
	});

	test("updates and removes query parameters without changing the route", () => {
		expect(
			resolveEmbeddedPageNavigation(
				{
					type: "setQueryParam",
					key: "status",
					value: "closed",
					replace: false,
				},
				"orders-app",
				{ status: "open", page: "2" },
			),
		).toEqual({ queryParams: { status: "closed", page: "2" } });

		expect(
			resolveEmbeddedPageNavigation(
				{
					type: "setQueryParam",
					key: "status",
					value: "",
					replace: true,
				},
				"orders-app",
				{ status: "open", page: "2" },
			),
		).toEqual({ queryParams: { page: "2" } });
	});

	test("hands external and cross-app destinations back as external links", () => {
		expect(
			resolveEmbeddedPageNavigation(
				{
					type: "navigateTo",
					route: "https://example.com/help",
					replace: false,
				},
				"orders-app",
				{ order: "42" },
			),
		).toEqual({
			externalHref: "https://example.com/help",
			queryParams: { order: "42" },
		});

		expect(
			resolveEmbeddedPageNavigation(
				{
					type: "navigateTo",
					route: "/use?id=another-app&route=/",
					replace: false,
				},
				"orders-app",
				{},
			),
		).toEqual({
			externalHref: "/use?id=another-app&route=/",
			queryParams: {},
		});
	});
});

describe("isSafeEmbeddedExternalHref", () => {
	test("allows links and rejects executable URL schemes", () => {
		expect(isSafeEmbeddedExternalHref("https://example.com/help")).toBe(true);
		expect(isSafeEmbeddedExternalHref("/use?id=another-app")).toBe(true);
		expect(isSafeEmbeddedExternalHref("mailto:support@example.com")).toBe(true);
		expect(isSafeEmbeddedExternalHref("javascript:alert(1)")).toBe(false);
		expect(isSafeEmbeddedExternalHref("data:text/html,unsafe")).toBe(false);
	});
});
