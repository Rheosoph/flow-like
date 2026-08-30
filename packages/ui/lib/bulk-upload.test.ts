import { describe, expect, test } from "bun:test";
import {
	BulkUploadHttpError,
	type IBulkUploadProgress,
	type IBulkUploadTask,
	assertBulkUploadSucceeded,
	buildUploadPath,
	isRetryableUploadError,
	runBulkUpload,
} from "./bulk-upload";

function fakeFile(name: string, size: number): File {
	return new File([new Uint8Array(size)], name);
}

function tasks(count: number, size = 10): IBulkUploadTask[] {
	return Array.from({ length: count }, (_, index) => ({
		path: `folder/file-${index}.bin`,
		file: fakeFile(`file-${index}.bin`, size),
	}));
}

/** Resolves every path to itself, recording the batch sizes it was handed. */
function echoPrepare(batchSizes: number[]) {
	return async (paths: readonly string[]) => {
		batchSizes.push(paths.length);
		return new Map(paths.map((path) => [path, path]));
	};
}

describe("runBulkUpload", () => {
	test("uploads every file across many batches, not just the first", async () => {
		const batchSizes: number[] = [];
		const sent: string[] = [];

		const result = await runBulkUpload<string>(
			tasks(250),
			{
				prepare: echoPrepare(batchSizes),
				send: async (target) => {
					sent.push(target);
				},
			},
			{ batchSize: 100, concurrency: 4 },
		);

		expect(sent).toHaveLength(250);
		expect(new Set(sent).size).toBe(250);
		expect(result.uploaded).toBe(250);
		expect(result.failed).toEqual([]);
		// 100 + 100 + 50 — never one oversized request.
		expect(batchSizes.reduce((a, b) => a + b, 0)).toBe(250);
		expect(Math.max(...batchSizes)).toBeLessThanOrEqual(100);
	});

	test("never exceeds the configured concurrency", async () => {
		let inFlight = 0;
		let peak = 0;

		await runBulkUpload<string>(
			tasks(60),
			{
				prepare: echoPrepare([]),
				send: async () => {
					inFlight += 1;
					peak = Math.max(peak, inFlight);
					await new Promise((resolve) => setTimeout(resolve, 1));
					inFlight -= 1;
				},
			},
			{ batchSize: 10, concurrency: 5 },
		);

		expect(peak).toBeLessThanOrEqual(5);
		expect(peak).toBeGreaterThan(1);
	});

	test("keeps the pool fed across batch boundaries instead of draining it", async () => {
		// A prepare that only ever runs while the queue is empty means every
		// batch boundary stalls the whole run for a round trip.
		let idleAtPrepare = 0;
		let inFlight = 0;

		await runBulkUpload<string>(
			tasks(60),
			{
				prepare: async (paths) => {
					if (inFlight === 0) idleAtPrepare += 1;
					await new Promise((resolve) => setTimeout(resolve, 3));
					return new Map(paths.map((p) => [p, p]));
				},
				send: async () => {
					inFlight += 1;
					await new Promise((resolve) => setTimeout(resolve, 2));
					inFlight -= 1;
				},
			},
			{ batchSize: 10, concurrency: 4 },
		);

		// Only the very first prepare should find nothing in flight.
		expect(idleAtPrepare).toBe(1);
	});

	// The prefetch claims a batch index synchronously but resolves it
	// asynchronously. When the batch it claims is the LAST one, a naive
	// "are there batches left?" check reads false while the resolve is still
	// in flight, every worker retires, and the run returns successfully with
	// that batch never uploaded — the same silent truncation this module
	// exists to prevent. These two cover the web and desktop-offline shapes;
	// both need a genuinely async `prepare` to expose it.
	test("uploads the final batch even when its prefetch is still in flight", async () => {
		const sent = new Set<string>();

		const result = await runBulkUpload<string>(
			tasks(1300),
			{
				prepare: async (paths) => {
					await new Promise((resolve) => setTimeout(resolve, 30));
					return new Map(paths.map((path) => [path, path]));
				},
				send: async (target) => {
					await new Promise((resolve) => setTimeout(resolve, 1));
					sent.add(target);
				},
			},
			{ batchSize: 100, concurrency: 8 },
		);

		expect(sent.size).toBe(1300);
		expect(result.uploaded).toBe(1300);
		expect(result.failed).toEqual([]);
	});

	test("uploads the final batch at the desktop offline batch/concurrency shape", async () => {
		const sent = new Set<string>();

		const result = await runBulkUpload<string>(
			tasks(100),
			{
				prepare: async (paths) => {
					await new Promise((resolve) => setTimeout(resolve, 8));
					return new Map(paths.map((path) => [path, path]));
				},
				send: async (target) => {
					await new Promise((resolve) => setTimeout(resolve, 1));
					sent.add(target);
				},
			},
			// LOCAL_WRITE_CONCURRENCY and LOCAL_WRITE_CONCURRENCY * 4.
			{ batchSize: 16, concurrency: 4 },
		);

		expect(sent.size).toBe(100);
		expect(result.uploaded).toBe(100);
	});

	test("a throwing progress callback neither retries nor double-counts a sent file", async () => {
		const sends: string[] = [];
		let progressCalls = 0;

		const result = await runBulkUpload<string>(
			tasks(5, 100),
			{
				prepare: echoPrepare([]),
				send: async (_target, task) => {
					sends.push(task.path);
				},
			},
			{
				concurrency: 1,
				maxAttempts: 3,
				onProgress: () => {
					progressCalls += 1;
					if (progressCalls === 2) throw new Error("render blew up");
				},
			},
		);

		expect(sends).toHaveLength(5);
		expect(new Set(sends).size).toBe(5);
		expect(result.uploaded).toBe(5);
		expect(result.failed).toEqual([]);
	});

	test("the returned failure list is a snapshot, not the live array", async () => {
		const result = await runBulkUpload<string>(
			tasks(4),
			{
				prepare: async (paths) =>
					new Map(
						paths
							.filter((path) => !path.endsWith("file-0.bin"))
							.map((path) => [path, path]),
					),
				send: async () => {},
			},
			{ batchSize: 2, concurrency: 2 },
		);

		const observed = result.failed.length;
		await new Promise((resolve) => setTimeout(resolve, 50));
		expect(result.failed.length).toBe(observed);
	});

	test("retries a retryable failure and succeeds", async () => {
		const attemptsByPath = new Map<string, number>();

		const result = await runBulkUpload<string>(
			tasks(3),
			{
				prepare: echoPrepare([]),
				send: async (_target, task) => {
					const attempts = (attemptsByPath.get(task.path) ?? 0) + 1;
					attemptsByPath.set(task.path, attempts);
					if (attempts < 2) throw new BulkUploadHttpError(503, "flaky");
				},
			},
			{ concurrency: 2, maxAttempts: 3 },
		);

		expect(result.failed).toEqual([]);
		expect(result.uploaded).toBe(3);
		for (const attempts of attemptsByPath.values()) expect(attempts).toBe(2);
	});

	test("does not retry a refusal, and records it without failing the run", async () => {
		let sendCalls = 0;

		const result = await runBulkUpload<string>(
			tasks(4),
			{
				prepare: echoPrepare([]),
				send: async (_target, task) => {
					sendCalls += 1;
					if (task.path.endsWith("file-1.bin")) {
						throw new BulkUploadHttpError(404, "gone");
					}
				},
			},
			{ concurrency: 1, maxAttempts: 3 },
		);

		expect(result.failed).toHaveLength(1);
		expect(result.failed[0].path).toBe("folder/file-1.bin");
		expect(result.uploaded).toBe(3);
		// Three successes plus exactly one attempt at the refusal.
		expect(sendCalls).toBe(4);
	});

	test("records a file the backend would not resolve and uploads the rest", async () => {
		const result = await runBulkUpload<string>(
			tasks(5),
			{
				prepare: async (paths) =>
					new Map(
						paths
							.filter((path) => !path.endsWith("file-2.bin"))
							.map((path) => [path, path]),
					),
				send: async () => {},
			},
			{ concurrency: 2 },
		);

		expect(result.uploaded).toBe(4);
		expect(result.failed).toHaveLength(1);
		expect(result.failed[0].path).toBe("folder/file-2.bin");
	});

	test("reports byte-weighted progress that ends at 100", async () => {
		const percents: number[] = [];
		const details: IBulkUploadProgress[] = [];

		// Sizes differ by 100x, so file-count progress would be visibly wrong.
		const mixed: IBulkUploadTask[] = [
			{ path: "a", file: fakeFile("a", 1) },
			{ path: "b", file: fakeFile("b", 1) },
			{ path: "c", file: fakeFile("c", 998) },
		];

		await runBulkUpload<string>(
			mixed,
			{
				prepare: echoPrepare([]),
				send: async (_target, task, onBytes) => {
					onBytes(task.file.size);
				},
			},
			{
				concurrency: 1,
				onProgress: (percent, detail) => {
					percents.push(percent);
					if (detail) details.push(detail);
				},
			},
		);

		expect(percents.at(-1)).toBe(100);
		const final = details.at(-1);
		expect(final?.phase).toBe("done");
		expect(final?.completedFiles).toBe(3);
		expect(final?.uploadedBytes).toBe(1000);
		expect(final?.failedFiles).toBe(0);
	});

	test("reaches 100 percent for a folder of empty files", async () => {
		const percents: number[] = [];

		await runBulkUpload<string>(
			tasks(4, 0),
			{ prepare: echoPrepare([]), send: async () => {} },
			{ concurrency: 2, onProgress: (percent) => percents.push(percent) },
		);

		expect(percents.at(-1)).toBe(100);
	});

	test("stops early when the caller aborts", async () => {
		const controller = new AbortController();
		let sendCalls = 0;

		const result = await runBulkUpload<string>(
			tasks(100),
			{
				prepare: echoPrepare([]),
				send: async () => {
					sendCalls += 1;
					if (sendCalls === 5) controller.abort();
					await new Promise((resolve) => setTimeout(resolve, 1));
				},
			},
			{ batchSize: 10, concurrency: 2, signal: controller.signal },
		);

		expect(result.cancelled).toBe(true);
		expect(sendCalls).toBeLessThan(100);
		// Files the cancellation never reached must not be counted as uploaded.
		expect(result.uploaded).toBeLessThanOrEqual(sendCalls);
	});

	test("propagates a prepare failure, since it condemns every remaining file", async () => {
		await expect(
			runBulkUpload<string>(
				tasks(10),
				{
					prepare: async () => {
						throw new Error("401 Unauthorized");
					},
					send: async () => {},
				},
				{ batchSize: 5 },
			),
		).rejects.toThrow("401 Unauthorized");
	});

	test("an empty file list is a no-op that still reports completion", async () => {
		let prepareCalls = 0;
		const result = await runBulkUpload<string>(
			[],
			{
				prepare: async () => {
					prepareCalls += 1;
					return new Map();
				},
				send: async () => {},
			},
			{},
		);

		expect(prepareCalls).toBe(0);
		expect(result.uploaded).toBe(0);
		expect(result.failed).toEqual([]);
	});
});

describe("assertBulkUploadSucceeded", () => {
	test("throws with a count when files failed, so no partial run reads as success", () => {
		expect(() =>
			assertBulkUploadSucceeded({
				uploaded: 8,
				failed: [
					{ path: "a.bin", error: "boom", attempts: 4 },
					{ path: "b.bin", error: "boom", attempts: 4 },
				],
				totalBytes: 0,
				uploadedBytes: 0,
				durationMs: 1,
				cancelled: false,
			}),
		).toThrow("2 of 10 files failed to upload");
	});

	test("is silent for a clean run", () => {
		expect(() =>
			assertBulkUploadSucceeded({
				uploaded: 3,
				failed: [],
				totalBytes: 0,
				uploadedBytes: 0,
				durationMs: 1,
				cancelled: false,
			}),
		).not.toThrow();
	});
});

describe("isRetryableUploadError", () => {
	test("retries transport faults and server backpressure", () => {
		for (const status of [0, 403, 408, 429, 500, 502, 503]) {
			expect(isRetryableUploadError(new BulkUploadHttpError(status, "x"))).toBe(
				true,
			);
		}
	});

	test("does not retry a refusal", () => {
		for (const status of [400, 401, 404, 409, 413]) {
			expect(isRetryableUploadError(new BulkUploadHttpError(status, "x"))).toBe(
				false,
			);
		}
	});
});

describe("buildUploadPath", () => {
	test("preserves folder structure from a directory pick", () => {
		const file = fakeFile("a.jpg", 1);
		Object.defineProperty(file, "webkitRelativePath", {
			value: "photos/2024/a.jpg",
		});
		expect(buildUploadPath("uploads", file)).toBe("uploads/photos/2024/a.jpg");
	});

	test("falls back to the file name and tolerates a slashed prefix", () => {
		const file = fakeFile("a.jpg", 1);
		expect(buildUploadPath("", file)).toBe("a.jpg");
		expect(buildUploadPath("/uploads/", file)).toBe("uploads/a.jpg");
	});
});
