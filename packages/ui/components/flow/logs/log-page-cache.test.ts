import { describe, expect, test } from "bun:test";
import { LogPageCache } from "./log-page-cache";

function rows(offset: number, limit: number) {
	return Array.from({ length: limit }, (_, i) => offset + i);
}

describe("LogPageCache", () => {
	test("maps row indices onto pages", () => {
		const cache = new LogPageCache<number>(200, 4);
		cache.set(1, rows(200, 200));
		expect(cache.row(250)).toBe(250);
		expect(cache.row(10)).toBeUndefined();
		expect(cache.pagesFor(150, 450, 1000)).toEqual([0, 1, 2]);
		expect(cache.pagesFor(0, 50, 30)).toEqual([0]);
		expect(cache.pagesFor(0, 10, 0)).toEqual([]);
	});

	test("evicts the least recently used page", () => {
		const cache = new LogPageCache<number>(10, 3);
		cache.set(0, rows(0, 10));
		cache.set(1, rows(10, 10));
		cache.set(2, rows(20, 10));
		cache.touch(0);
		cache.set(3, rows(30, 10));
		expect(cache.has(0)).toBe(true);
		expect(cache.has(1)).toBe(false);
		expect(cache.size).toBe(3);
	});

	test("dedupes a page that is already loading", async () => {
		const cache = new LogPageCache<number>(10, 3);
		const calls: Array<[number, number]> = [];
		const fetchPage = async (offset: number, limit: number) => {
			calls.push([offset, limit]);
			return rows(offset, limit);
		};
		const first = cache.request(2, fetchPage);
		expect(cache.request(2, fetchPage)).toBeUndefined();
		expect(cache.isLoading(2)).toBe(true);
		await first;
		expect(calls).toEqual([[20, 10]]);
		expect(cache.isLoading(2)).toBe(false);
		expect(cache.row(25)).toBe(25);
		expect(cache.request(2, fetchPage)).toBeUndefined();
	});

	test("remembers a failed page until it loads", async () => {
		const cache = new LogPageCache<number>(10, 3);
		await cache
			.request(0, async () => {
				throw new Error("not implemented");
			})
			?.catch(() => undefined);
		expect(cache.hasFailed(0)).toBe(true);
		expect(cache.isLoading(0)).toBe(false);
		await cache.request(0, async (offset, limit) => rows(offset, limit));
		expect(cache.hasFailed(0)).toBe(false);
	});
});
