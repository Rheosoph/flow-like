import { describe, expect, test } from "bun:test";
import {
	hasPageActionCapability,
	pageSurfaceCacheKey,
	pageSurfaceQueryKey,
	pageSurfaceRevision,
	pageSurfaceRouteKey,
	selectEvictions,
} from "./page-surface-cache";

describe("page surface capability storage", () => {
	test("detects dynamic Page capabilities at any component depth", () => {
		expect(
			hasPageActionCapability({
				components: {
					button: {
						actions: [
							{
								pageAction: {
									actionId: "da1_action",
									capabilityJwt: "signed-value",
								},
							},
						],
					},
				},
			}),
		).toBe(true);
	});

	test("allows static opaque action references", () => {
		expect(
			hasPageActionCapability({
				pageAction: {
					actionId: "pa1_action",
					manifestRevision: "per1_revision",
				},
			}),
		).toBe(false);
	});

	test("detects capabilities nested inside literalJson bindings", () => {
		expect(
			hasPageActionCapability({
				data: {
					literalJson: JSON.stringify({
						rows: [
							{
								action: {
									pageAction: {
										actionId: "da1_nested",
										capabilityJwt: "signed-value",
									},
								},
							},
						],
					}),
				},
			}),
		).toBe(true);
	});

	test("detects snake-case capabilities inside literal_json bindings", () => {
		expect(
			hasPageActionCapability({
				data: {
					literal_json: JSON.stringify({
						page_action: {
							action_id: "da1_nested",
							capability_jwt: "signed-value",
						},
					}),
				},
			}),
		).toBe(true);
	});

	test("rejects native dynamic Page actions without a JWT", () => {
		expect(
			hasPageActionCapability({
				component: {
					actions: [
						{
							pageAction: {
								actionId: "lda1_native-runtime-action",
								manifestRevision: "per1_revision",
							},
						},
					],
				},
			}),
		).toBe(true);
	});

	test("rejects native dynamic Page actions encoded in literal JSON", () => {
		expect(
			hasPageActionCapability({
				data: {
					literal_json: JSON.stringify({
						page_action: {
							action_id: "lda1_native-runtime-binding",
						},
					}),
				},
			}),
		).toBe(true);
	});

	test("does not interpret arbitrary strings as executable JSON", () => {
		const encodedCapability = JSON.stringify({
			pageAction: {
				actionId: "da1_nested",
				capabilityJwt: "signed-value",
			},
		});
		expect(hasPageActionCapability({ label: encodedCapability })).toBe(false);
		expect(hasPageActionCapability({ literalJson: "not-json" })).toBe(false);
	});
});

describe("page surface query signature", () => {
	test("is independent of parameter order", () => {
		expect(pageSurfaceQueryKey("?b=2&a=1")).toBe(
			pageSurfaceQueryKey("a=1&b=2"),
		);
	});

	test("separates pages opened with different parameters", () => {
		expect(pageSurfaceQueryKey("?item=1")).not.toBe(
			pageSurfaceQueryKey("?item=2"),
		);
		expect(pageSurfaceQueryKey("?item=1")).not.toBe(pageSurfaceQueryKey(""));
	});

	test("treats no parameters and no search string alike", () => {
		expect(pageSurfaceQueryKey(undefined)).toBe("");
		expect(pageSurfaceQueryKey("?")).toBe("");
	});
});

describe("page surface authority revision", () => {
	test("changes when either Page content or execution authority changes", () => {
		const current = pageSurfaceRevision("page-revision", "per2_current");
		expect(pageSurfaceRevision("other-page", "per2_current")).not.toBe(current);
		expect(pageSurfaceRevision("page-revision", "per2_other")).not.toBe(
			current,
		);
	});

	test("keeps legacy and preview identities compatible without authority", () => {
		expect(pageSurfaceRevision("page-revision")).toBe("page-revision");
		expect(pageSurfaceRevision(undefined, "per2_current")).toBeUndefined();
	});
});

describe("page surface route signature", () => {
	test("normalizes equivalent route spellings", () => {
		expect(pageSurfaceRouteKey("settings/?tab=api#keys")).toBe("/settings");
		expect(pageSurfaceRouteKey("/settings")).toBe("/settings");
		expect(pageSurfaceRouteKey(undefined)).toBe("/");
	});

	test("separates different route inputs", () => {
		expect(pageSurfaceRouteKey("/orders")).not.toBe(
			pageSurfaceRouteKey("/customers"),
		);
	});
});

describe("page surface identity key", () => {
	const identity = {
		appId: "app",
		pageId: "page",
		pageUpdatedAt: "2026-08-31T10:00:00Z",
		routeKey: "/orders",
		queryKey: "item=1",
		userKey: "user-a",
	};

	test("changes for every freshness and isolation boundary", () => {
		const base = pageSurfaceCacheKey(identity);
		for (const changed of [
			{ ...identity, appId: "other-app" },
			{ ...identity, pageId: "other-page" },
			{ ...identity, pageUpdatedAt: "2026-08-31T10:00:01Z" },
			{ ...identity, routeKey: "/customers" },
			{ ...identity, queryKey: "item=2" },
			{ ...identity, userKey: "user-b" },
		]) {
			expect(pageSurfaceCacheKey(changed)).not.toBe(base);
		}
	});
});

describe("page surface eviction", () => {
	const prefix = "app page user ";
	const keyFor = (revision: string, query: string) =>
		`${prefix}${revision}${" "}${JSON.stringify(["/route", query])}`;

	test("drops the page's superseded revisions", () => {
		const current = keyFor("2026-08-09T10:00:00Z", "item=1");
		const manifest = {
			[keyFor("2026-08-01T10:00:00Z", "item=1")]: 1,
			[keyFor("2026-08-01T10:00:00Z", "item=2")]: 2,
			[current]: 3,
		};

		expect(
			selectEvictions(
				manifest,
				current,
				prefix,
				"2026-08-09T10:00:00Z",
			).toSorted(),
		).toEqual(
			[
				keyFor("2026-08-01T10:00:00Z", "item=1"),
				keyFor("2026-08-01T10:00:00Z", "item=2"),
			].toSorted(),
		);
	});

	test("keeps every entry of the current revision while under budget", () => {
		const revision = "2026-08-09T10:00:00Z";
		const current = keyFor(revision, "item=1");
		const manifest = {
			[current]: 2,
			[keyFor(revision, "item=2")]: 1,
			"other-app page user rev q": 0,
		};

		expect(selectEvictions(manifest, current, prefix, revision)).toEqual([]);
	});

	test("evicts oldest first past the budget and never the entry just written", () => {
		const revision = "rev";
		const current = keyFor(revision, "item=0");
		const manifest: Record<string, number> = { [current]: 10_000 };
		// 60 unrelated pages, oldest first, well past the 48-entry budget.
		for (let index = 0; index < 60; index += 1) {
			manifest[`app-${index} page user rev q`] = index;
		}

		const evicted = selectEvictions(manifest, current, prefix, revision);

		expect(evicted).not.toContain(current);
		expect(Object.keys(manifest).length - evicted.length).toBe(48);
		expect(evicted).toContain("app-0 page user rev q");
		expect(evicted).not.toContain("app-59 page user rev q");
	});
});
