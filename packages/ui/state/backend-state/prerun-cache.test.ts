import {
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import {
	invalidatePrerunCache,
	prerunBoardKey,
	prerunEventKey,
	prerunSwr,
} from "./prerun-cache";

interface Prerun {
	signature: string;
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const settleBackground = () =>
	new Promise<void>((resolve) => setImmediate(resolve));

let now = 0;

beforeEach(() => {
	invalidatePrerunCache();
	now = 1_000;
	spyOn(Date, "now").mockImplementation(() => now);
});

afterEach(() => {
	invalidatePrerunCache();
	mock.restore();
});

describe("prerunSwr", () => {
	test("shares one cold request across concurrent callers", async () => {
		const response = deferred<Prerun>();
		const fetcher = mock(() => response.promise);
		const requests = Array.from({ length: 12 }, () =>
			prerunSwr("key", fetcher),
		);
		await Promise.resolve();
		expect(fetcher).toHaveBeenCalledTimes(1);

		const data = { signature: "first" };
		response.resolve(data);
		expect(await Promise.all(requests)).toEqual(Array(12).fill(data));
		expect(await prerunSwr("key", fetcher)).toBe(data);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	test("keeps apps, targets, and versions in separate requests", async () => {
		const keys = [
			prerunBoardKey("app", "target"),
			prerunBoardKey("other-app", "target"),
			prerunBoardKey("app", "other-target"),
			prerunBoardKey("app", "target", [1, 0, 0]),
			prerunEventKey("app", "target"),
		];
		const results = await Promise.all(
			keys.map((key) => prerunSwr(key, async () => ({ signature: key }))),
		);
		expect(results.map((result) => result.signature)).toEqual(keys);
	});

	test("starts the freshness window when a slow response arrives", async () => {
		const response = deferred<Prerun>();
		const fetcher = mock(() => response.promise);
		const first = prerunSwr("key", fetcher);
		now += 20_000;
		response.resolve({ signature: "first" });
		await first;

		now += 14_000;
		await prerunSwr("key", fetcher);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	test("rejects all waiting callers and retries a failed cold request", async () => {
		const response = deferred<Prerun>();
		const fetcher = mock(() => response.promise);
		const first = prerunSwr("key", fetcher);
		const second = prerunSwr("key", fetcher);
		const error = new Error("offline");
		const results = Promise.allSettled([first, second]);
		response.reject(error);
		expect(await results).toEqual([
			{ status: "rejected", reason: error },
			{ status: "rejected", reason: error },
		]);
		expect(fetcher).toHaveBeenCalledTimes(1);

		const fresh = { signature: "retry" };
		expect(await prerunSwr("key", async () => fresh)).toBe(fresh);
	});

	test("clears a cold request when its fetcher throws synchronously", async () => {
		await expect(
			prerunSwr("key", () => {
				throw new Error("offline");
			}),
		).rejects.toThrow("offline");
		const fresh = { signature: "retry" };
		expect(await prerunSwr("key", async () => fresh)).toBe(fresh);
	});

	test.each([
		["undefined", undefined],
		["null", null],
		["array", []],
	])(
		"does not cache a malformed cold response: %s",
		async (_name, malformed) => {
			const fetcher = mock(async () => malformed as unknown as Prerun);
			const result: unknown = await prerunSwr("key", fetcher);
			expect(result).toBe(malformed);
			await prerunSwr("key", fetcher);
			expect(fetcher).toHaveBeenCalledTimes(2);
		},
	);

	test("returns stale data during one refresh and reports signature drift", async () => {
		const stale = { signature: "old" };
		await prerunSwr("key", async () => stale);
		now += 16_000;
		const response = deferred<Prerun>();
		const fetcher = mock(() => response.promise);
		const onDrift = mock();

		expect(await prerunSwr("key", fetcher, { onDrift })).toBe(stale);
		expect(await prerunSwr("key", fetcher, { onDrift })).toBe(stale);
		expect(fetcher).toHaveBeenCalledTimes(1);

		const fresh = { signature: "new" };
		response.resolve(fresh);
		await settleBackground();
		expect(await prerunSwr("key", fetcher)).toBe(fresh);
		expect(onDrift).toHaveBeenCalledTimes(1);
		expect(onDrift).toHaveBeenCalledWith("key", stale, fresh);
	});

	test.each(["rejection", "synchronous throw", "malformed response"])(
		"retains stale data and retries after a background %s",
		async (failure) => {
			const warn = spyOn(console, "warn").mockImplementation(() => {});
			const stale = { signature: "old" };
			await prerunSwr("key", async () => stale);
			now += 16_000;
			expect(
				await prerunSwr("key", () => {
					if (failure === "synchronous throw") throw new Error("offline");
					if (failure === "rejection")
						return Promise.reject(new Error("offline"));
					return Promise.resolve(null as unknown as Prerun);
				}),
			).toBe(stale);
			await settleBackground();
			expect(warn).toHaveBeenCalledTimes(1);

			const fresh = { signature: "retry" };
			const retry = mock(async () => fresh);
			expect(await prerunSwr("key", retry)).toBe(stale);
			await settleBackground();
			expect(await prerunSwr("key", retry)).toBe(fresh);
			expect(retry).toHaveBeenCalledTimes(1);
		},
	);

	test.each(["key", "all"])(
		"detaches a cold request after invalidating %s entries",
		async (scope) => {
			const obsolete = deferred<Prerun>();
			const first = prerunSwr("key", () => obsolete.promise);
			invalidatePrerunCache(scope === "all" ? undefined : "key");
			const current = deferred<Prerun>();
			const fetcher = mock(() => current.promise);
			const second = prerunSwr("key", fetcher);

			const oldData = { signature: "old" };
			obsolete.resolve(oldData);
			expect(await first).toBe(oldData);
			const third = prerunSwr("key", fetcher);
			const fresh = { signature: "new" };
			current.resolve(fresh);
			expect(await Promise.all([second, third])).toEqual([fresh, fresh]);
			expect(fetcher).toHaveBeenCalledTimes(1);
			expect(await prerunSwr("key", fetcher)).toBe(fresh);
		},
	);

	test("ignores an invalidated background result after its replacement arrives", async () => {
		await prerunSwr("key", async () => ({ signature: "old" }));
		now += 16_000;
		const obsolete = deferred<Prerun>();
		const onDrift = mock();
		await prerunSwr("key", () => obsolete.promise, { onDrift });
		invalidatePrerunCache("key");

		const fresh = { signature: "new" };
		await prerunSwr("key", async () => fresh);
		obsolete.resolve({ signature: "obsolete" });
		await settleBackground();
		expect(
			await prerunSwr("key", async () => ({ signature: "unexpected" })),
		).toBe(fresh);
		expect(onDrift).not.toHaveBeenCalled();
	});

	test("an invalidated background failure cannot clear a newer refresh", async () => {
		spyOn(console, "warn").mockImplementation(() => {});
		await prerunSwr("key", async () => ({ signature: "old" }));
		now += 16_000;
		const obsolete = deferred<Prerun>();
		await prerunSwr("key", () => obsolete.promise);
		invalidatePrerunCache("key");
		await prerunSwr("key", async () => ({ signature: "replacement" }));
		now += 16_000;
		const current = deferred<Prerun>();
		const fetcher = mock(() => current.promise);
		await prerunSwr("key", fetcher);

		obsolete.reject(new Error("offline"));
		await settleBackground();
		await prerunSwr("key", fetcher);
		expect(fetcher).toHaveBeenCalledTimes(1);
		current.resolve({ signature: "new" });
		await settleBackground();
	});
});
