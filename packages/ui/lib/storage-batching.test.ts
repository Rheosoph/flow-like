import { describe, expect, test } from "bun:test";
import {
	MAX_STORAGE_PREFIXES_PER_REQUEST,
	chunkPrefixes,
	requestPrefixesInBatches,
} from "./storage-batching";

function prefixes(count: number): string[] {
	return Array.from({ length: count }, (_, index) => `file-${index}.bin`);
}

describe("chunkPrefixes", () => {
	test("splits at the server cap and keeps every prefix exactly once", () => {
		const batches = chunkPrefixes(prefixes(250));
		expect(batches.map((b) => b.length)).toEqual([100, 100, 50]);
		expect(batches.flat()).toEqual(prefixes(250));
	});

	test("leaves a batch at or under the cap alone", () => {
		expect(
			chunkPrefixes(prefixes(MAX_STORAGE_PREFIXES_PER_REQUEST)),
		).toHaveLength(1);
		expect(chunkPrefixes([])).toEqual([]);
	});
});

describe("requestPrefixesInBatches", () => {
	test("never asks for more than the route accepts", async () => {
		const sizes: number[] = [];

		const results = await requestPrefixesInBatches(
			prefixes(250),
			async (batch) => {
				sizes.push(batch.length);
				return batch.map((prefix) => ({ prefix, url: `https://x/${prefix}` }));
			},
		);

		expect(Math.max(...sizes)).toBeLessThanOrEqual(
			MAX_STORAGE_PREFIXES_PER_REQUEST,
		);
		expect(results).toHaveLength(250);
	});

	test("returns results in request order despite concurrent batches", async () => {
		const results = await requestPrefixesInBatches(
			prefixes(250),
			async (batch) => {
				// Later batches finish first, so ordering cannot come from timing.
				await new Promise((resolve) =>
					setTimeout(resolve, batch[0] === "file-0.bin" ? 20 : 1),
				);
				return batch.map((prefix) => ({ prefix }));
			},
		);

		expect(results.map((r) => r.prefix)).toEqual(prefixes(250));
	});

	test("a failed batch degrades to per-prefix errors without discarding the rest", async () => {
		const results = await requestPrefixesInBatches(
			prefixes(250),
			async (batch) => {
				if (batch[0] === "file-100.bin") throw new Error("500");
				return batch.map((prefix) => ({ prefix, url: "ok" }));
			},
			{ errorMessage: "Download failed" },
		);

		expect(results).toHaveLength(250);
		const failures = results.filter((r) => r.error);
		expect(failures).toHaveLength(100);
		expect(failures[0].error).toBe("Download failed");
		expect(results.filter((r) => r.url)).toHaveLength(150);
	});

	test("issues a single request when the batch already fits", async () => {
		let calls = 0;
		await requestPrefixesInBatches(prefixes(100), async (batch) => {
			calls += 1;
			return batch.map((prefix) => ({ prefix }));
		});
		expect(calls).toBe(1);
	});

	test("makes no request at all for an empty selection", async () => {
		let calls = 0;
		const results = await requestPrefixesInBatches([], async () => {
			calls += 1;
			return [];
		});
		expect(calls).toBe(0);
		expect(results).toEqual([]);
	});
});
