import { describe, expect, test } from "bun:test";
import {
	pageSurfaceCacheKey,
	pageSurfaceQueryKey,
	selectEvictions,
} from "./page-surface-cache";

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

describe("page surface identity key", () => {
	const identity = {
		appId: "app",
		pageId: "page",
		pageUpdatedAt: "2026-08-31T10:00:00Z",
		queryKey: "item=1",
		userKey: "user-a",
	};

	test("changes for every freshness and isolation boundary", () => {
		const base = pageSurfaceCacheKey(identity);
		for (const changed of [
			{ ...identity, appId: "other-app" },
			{ ...identity, pageId: "other-page" },
			{ ...identity, pageUpdatedAt: "2026-08-31T10:00:01Z" },
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
		`${prefix}${revision}${" "}${query}`;

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
