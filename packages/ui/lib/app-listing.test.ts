import { describe, expect, test } from "bun:test";
import { fetchAllAppPages } from "./app-listing";

const entries = (count: number, start = 0) =>
	Array.from({ length: count }, (_, index) => [
		{ id: `app-${start + index}`, visibility: "Private" },
		{ name: `App ${start + index}` },
	]);

describe("fetchAllAppPages", () => {
	test("reads an empty final page when the list fills exactly two pages", async () => {
		const apps = entries(200);
		const offsets: number[] = [];
		const result = await fetchAllAppPages(async (offset, limit) => {
			offsets.push(offset);
			return apps.slice(offset, offset + limit);
		});
		expect<unknown>(result).toEqual(apps);
		expect(offsets).toEqual([0, 100, 200]);
	});

	test("deduplicates overlapping pages without dropping the following apps", async () => {
		const pages = [entries(100), entries(100, 95), entries(5, 195)];
		const result = await fetchAllAppPages(
			async (offset) => pages[offset / 100],
		);
		expect<unknown>(result).toEqual(entries(200));
	});

	test("rejects a failed later page instead of reporting a partial library", async () => {
		const failure = new Error("Hub unavailable");
		await expect(
			fetchAllAppPages(async (offset) => {
				if (offset > 0) throw failure;
				return entries(100);
			}),
		).rejects.toBe(failure);
	});

	test("keeps apps whose metadata is absent", async () => {
		const apps = [[{ id: "no-metadata" }, null], [{ id: "missing" }]];
		await expect<Promise<unknown>>(
			fetchAllAppPages(async () => apps),
		).resolves.toEqual(apps);
	});

	test.each([undefined, {}, [[null, null]], [[{ id: "app" }, "invalid"]]])(
		"rejects malformed pages: %j",
		async (page) => {
			await expect(fetchAllAppPages(async () => page)).rejects.toThrow(
				"invalid page",
			);
		},
	);

	test("fails when the server ignores the offset instead of paging forever", async () => {
		let calls = 0;
		await expect(
			fetchAllAppPages(async () => {
				calls += 1;
				return entries(100);
			}),
		).rejects.toThrow("repeated an earlier page");
		expect(calls).toBe(2);
	});
});
